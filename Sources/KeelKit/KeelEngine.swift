import Foundation

/// Thread-safe latch: `set()` returns true exactly once, from any thread.
/// Guards the transfer's terminal continuation against a double-resume when
/// both a progress-error and the done callback race.
final class OnceFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var fired = false
    func set() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if fired { return false }
        fired = true
        return true
    }
}

/// Async facade over the keel MTP kernel.
///
/// One engine per process. Calls are serialized twice over: this actor admits
/// one operation at a time, and the kernel itself holds an MTP session lock —
/// the same discipline the kernel's single MTP session enforces.
///
/// The Go exports block their calling thread until the result callback has
/// fired, so invocations run on a dedicated dispatch queue rather than on the
/// cooperative thread pool.
public actor KeelEngine {
    private let library: KeelLibrary
    private let ffiQueue = DispatchQueue(label: "keel.ffi", qos: .userInitiated)
    public private(set) var isConnected = false

    // Exactly one FFI operation may be in flight at a time. This is not
    // optional: the C ABI has no user-data pointer, so callback results route
    // through process-global slots (KeelCallbackSlots) that assume a single
    // owner — AND the kernel holds one MTP session that can only run one
    // transaction at a time. Actor methods interleave at every `await`, and the
    // app legitimately issues concurrent calls (a storage switch fires a
    // navigate while connect() is mid-walk; the UI stays live during a
    // transfer). Without an explicit gate that spans `await`, two operations
    // share the slots and a wakeup is lost — the hang seen at "Reading storage".
    private var ffiBusy = false
    private var ffiWaiters: [CheckedContinuation<Void, Never>] = []

    private func acquireFFI() async {
        while ffiBusy {
            await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
                ffiWaiters.append(c)
            }
        }
        ffiBusy = true
    }

    private func releaseFFI() {
        ffiBusy = false
        if !ffiWaiters.isEmpty {
            ffiWaiters.removeFirst().resume()
        }
    }

    public init(libraryDirectory: URL? = nil) throws {
        let dir = libraryDirectory ?? KeelLibrary.defaultLibraryDirectory()
        self.library = try KeelLibrary(directory: dir)
    }

    public var libraryPath: String { library.libraryPath }

    /// True when the loaded kernel has Ferry's `CancelTransfer` extension.
    public var supportsCancellation: Bool { library.supportsCancellation }

    /// Flags the in-flight transfer for cancellation; the transfer stream
    /// finishes with `KeelError.isCancellation`. Non-blocking — safe to call
    /// while the FFI queue is busy with the transfer itself.
    public func cancelActiveTransfer() {
        library.rawCancelTransfer()
    }

    // MARK: - Session

    /// Detects the connected MTP device and opens a session.
    @discardableResult
    public func initialize() async throws -> KeelDeviceInfo {
        let info: KeelDeviceInfo = try await call(.initialize)
        isConnected = true
        return info
    }

    public func deviceInfo() async throws -> KeelDeviceInfo {
        try await call(.fetchDeviceInfo)
    }

    public func storages() async throws -> [KeelStorage] {
        try await call(.fetchStorages)
    }

    /// Closes the MTP session. Safe to call when already disconnected.
    public func dispose() async {
        struct Ignored: Decodable, Sendable {}
        _ = try? await call(.dispose) as Ignored
        isConnected = false
    }

    // MARK: - Browsing

    public func list(
        storageId: UInt32, path: String, skipHiddenFiles: Bool = true,
        recursive: Bool = false
    ) async throws -> [KeelFile] {
        // keel returns `"data": null` for an empty directory (a preserved
        // quirk), so a walk that finds nothing must decode to [] — not throw
        // (which left empty folders stuck on the loading spinner forever).
        try await listCall(
            input: [
                "storageId": storageId,
                "fullPath": path,
                "recursive": recursive,
                "skipDisallowedFiles": false,
                "skipHiddenFiles": skipHiddenFiles,
            ])
    }

    public struct FileExistence: Decodable, Sendable {
        public let fullpath: String
        public let exists: Bool
    }

    public func filesExist(storageId: UInt32, paths: [String]) async throws -> [FileExistence] {
        try await call(.fileExists, input: ["storageId": storageId, "files": paths])
    }

    public var supportsThumbnails: Bool { library.supportsThumbnails }

    /// Device-generated thumbnail (JPEG bytes) for a file, or nil when the
    /// object has none / the kernel doesn't support GetThumb. Best-effort — it
    /// swallows errors so a broken thumbnail never disrupts browsing.
    public func thumbnail(storageId: UInt32, path: String) async -> Data? {
        guard library.supportsThumbnails else { return nil }
        let json = Self.encodeJSON(["storageId": storageId, "fullPath": path])
        await acquireFFI()
        defer { releaseFFI() }

        let payload = await withCheckedContinuation { continuation in
            KeelCallbackSlots.install { p in
                KeelCallbackSlots.clear()
                continuation.resume(returning: p)
            }
            ffiQueue.async { _ = self.library.rawFetchThumbnail(json: json) }
        }

        struct ThumbEnvelope: Decodable { let error: String?; let data: String? }
        guard let env = try? JSONDecoder().decode(
            ThumbEnvelope.self, from: Data(payload.utf8)),
            (env.error ?? "").isEmpty, let b64 = env.data
        else { return nil }
        return Data(base64Encoded: b64)
    }

    public func makeDirectory(storageId: UInt32, path: String) async throws {
        struct Done: Decodable, Sendable {}
        _ = try await call(
            .makeDirectory, input: ["storageId": storageId, "fullPath": path]
        ) as Done
    }

    public func rename(storageId: UInt32, path: String, to newName: String) async throws {
        struct Done: Decodable, Sendable {}
        _ = try await call(
            .renameFile,
            input: ["storageId": storageId, "fullPath": path, "newFileName": newName]
        ) as Done
    }

    public func delete(storageId: UInt32, paths: [String]) async throws {
        struct Done: Decodable, Sendable {}
        _ = try await call(.deleteFile, input: ["storageId": storageId, "files": paths])
            as Done
    }

    // MARK: - Transfers

    /// Streams preprocess + progress events, finishing after `.completed`.
    /// `download` copies phone → Mac, `upload` copies Mac → phone.
    public func transfer(
        _ direction: KeelTransferDirection,
        storageId: UInt32,
        sources: [String],
        destination: String,
        preprocessFiles: Bool = true
    ) -> AsyncThrowingStream<KeelTransferEvent, Error> {
        let input: [String: any Sendable] = [
            "storageId": storageId,
            "sources": sources,
            "destination": destination,
            "preprocessFiles": preprocessFiles,
        ]

        return AsyncThrowingStream { continuation in
            // Drive the transfer from an actor-isolated task so it can hold the
            // FFI gate (acquireFFI) for the transfer's full lifetime — no browse
            // op can steal the callback slots mid-transfer.
            let producer = Task { await self.runTransfer(direction, input: input, into: continuation) }
            continuation.onTermination = { _ in producer.cancel() }
        }
    }

    private func runTransfer(
        _ direction: KeelTransferDirection,
        input: [String: any Sendable],
        into continuation: AsyncThrowingStream<KeelTransferEvent, Error>.Continuation
    ) async {
        await acquireFFI()
        defer {
            KeelCallbackSlots.clear()
            releaseFFI()
        }

        let library = self.library
        let queue = self.ffiQueue
        let json = Self.encodeJSON(input)
        let decoder = JSONDecoder()

        // A single terminal continuation resumed exactly once — on done or on
        // the first error — so the gate is released precisely when the kernel
        // is finished with the device.
        await withCheckedContinuation { (finished: CheckedContinuation<Void, Never>) in
            let done = OnceFlag()
            KeelCallbackSlots.install(
                done: { payload in
                    if let envelope = try? decoder.decode(
                        KeelEnvelope<KeelJSONValue>.self, from: Data(payload.utf8)),
                        let failure = envelope.failure
                    {
                        continuation.finish(throwing: failure)
                    } else {
                        continuation.yield(.completed)
                        continuation.finish()
                    }
                    if done.set() { finished.resume() }
                },
                preprocess: { payload in
                    guard
                        let envelope = try? decoder.decode(
                            KeelEnvelope<KeelTransferPreprocess>.self,
                            from: Data(payload.utf8))
                    else { return }
                    if let failure = envelope.failure {
                        continuation.finish(throwing: failure)
                        if done.set() { finished.resume() }
                    } else if let data = envelope.data {
                        continuation.yield(.preprocess(data))
                    }
                },
                progress: { payload in
                    guard
                        let envelope = try? decoder.decode(
                            KeelEnvelope<KeelTransferProgress>.self,
                            from: Data(payload.utf8))
                    else { return }
                    if let failure = envelope.failure {
                        continuation.finish(throwing: failure)
                        if done.set() { finished.resume() }
                    } else if let data = envelope.data {
                        continuation.yield(.progress(data))
                    }
                }
            )

            queue.async {
                library.rawTransfer(direction, json: json)
            }
        }
    }

    // MARK: - Plumbing

    private func call<T: Decodable & Sendable>(_ fn: KeelSimpleFunction) async throws -> T {
        try await roundTrip { library, queue, resume in
            KeelCallbackSlots.install(done: resume)
            queue.async { library.rawCall(fn) }
        }
    }

    private func call<T: Decodable & Sendable>(
        _ fn: KeelJSONFunction, input: [String: any Sendable]
    ) async throws -> T {
        let json = Self.encodeJSON(input)
        return try await roundTrip { library, queue, resume in
            KeelCallbackSlots.install(done: resume)
            queue.async { library.rawCall(fn, json: json) }
        }
    }

    /// Walk specifically: `"data": null` (empty directory) decodes to [].
    private func listCall(input: [String: any Sendable]) async throws -> [KeelFile] {
        let json = Self.encodeJSON(input)
        await acquireFFI()
        defer { releaseFFI() }

        let payload = await withCheckedContinuation { continuation in
            KeelCallbackSlots.install { p in
                KeelCallbackSlots.clear()
                continuation.resume(returning: p)
            }
            ffiQueue.async { self.library.rawCall(.walk, json: json) }
        }

        let envelope: KeelEnvelope<[KeelFile]>
        do {
            envelope = try JSONDecoder().decode(
                KeelEnvelope<[KeelFile]>.self, from: Data(payload.utf8))
        } catch {
            throw KeelError.malformedPayload(payload)
        }
        if let failure = envelope.failure { throw failure }
        return envelope.data ?? []  // null data = empty directory
    }

    private func roundTrip<T: Decodable & Sendable>(
        _ start: (KeelLibrary, DispatchQueue, @escaping @Sendable (String) -> Void) -> Void
    ) async throws -> T {
        await acquireFFI()
        defer { releaseFFI() }

        let payload = await withCheckedContinuation { continuation in
            start(library, ffiQueue) { payload in
                KeelCallbackSlots.clear()
                continuation.resume(returning: payload)
            }
        }

        let envelope: KeelEnvelope<T>
        do {
            envelope = try JSONDecoder().decode(
                KeelEnvelope<T>.self, from: Data(payload.utf8))
        } catch {
            throw KeelError.malformedPayload(payload)
        }
        if let failure = envelope.failure { throw failure }
        guard let data = envelope.data else {
            throw KeelError.malformedPayload(payload)
        }
        return data
    }

    private static func encodeJSON(_ dictionary: [String: any Sendable]) -> String {
        guard
            let data = try? JSONSerialization.data(
                withJSONObject: dictionary, options: [.sortedKeys])
        else { return "{}" }
        return String(decoding: data, as: UTF8.self)
    }
}
