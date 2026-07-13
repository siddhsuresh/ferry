import Foundation

// MARK: - C ABI
//
// keel.dylib is the Rust MTP kernel. Every export takes an optional JSON
// input string plus one or more result callbacks:
//
//     typedef void (*on_cb_result_t)(char*);
//     void Initialize(on_cb_result_t* onDonePtr);
//     void Walk(char* json, on_cb_result_t* onDonePtr);
//     void DownloadFiles(char* json, on_cb_result_t* onPreprocess,
//                        on_cb_result_t* onProgress, on_cb_result_t* onDone);
//
// Note the header shape: parameters are declared `on_cb_result_t*`, but the
// kernel treats the parameter *value* as the function pointer itself. At the
// ABI level both are one pointer-sized argument, so we declare the effective
// signature and pass the callback directly.
//
// Callbacks fire synchronously on the calling thread before the export
// returns, so every call below is blocking. The engine dispatches them onto a
// dedicated serial queue.

typealias KeelCCallback = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void

private typealias FnCB = @convention(c) (KeelCCallback?) -> Void
private typealias FnJSONCB = @convention(c) (
    UnsafeMutablePointer<CChar>?, KeelCCallback?
) -> Void
private typealias FnJSON3CB = @convention(c) (
    UnsafeMutablePointer<CChar>?, KeelCCallback?, KeelCCallback?, KeelCCallback?
) -> Void
private typealias FnJSON2CB = @convention(c) (
    UnsafeMutablePointer<CChar>?, KeelCCallback?, KeelCCallback?
) -> Void
private typealias FnVoid = @convention(c) () -> Void

// MARK: - Callback slots
//
// C callbacks can't capture Swift context and the C ABI has no user-data
// pointer, so results are routed through process-global slots. The kernel
// holds a session lock (one MTP operation at a time) and `KeelEngine` is an
// actor that serializes calls, so a single set of slots is sufficient — the
// same one-op-at-a-time invariant the kernel enforces.

enum KeelCallbackSlots {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var done: (@Sendable (String) -> Void)?
    nonisolated(unsafe) private static var preprocess: (@Sendable (String) -> Void)?
    nonisolated(unsafe) private static var progress: (@Sendable (String) -> Void)?

    static func install(
        done: (@Sendable (String) -> Void)?,
        preprocess: (@Sendable (String) -> Void)? = nil,
        progress: (@Sendable (String) -> Void)? = nil
    ) {
        lock.lock()
        defer { lock.unlock() }
        self.done = done
        self.preprocess = preprocess
        self.progress = progress
    }

    static func clear() { install(done: nil) }

    fileprivate static func fire(
        _ keyPath: KeyPath<SlotView, (@Sendable (String) -> Void)?>,
        _ payload: String
    ) {
        lock.lock()
        let handler = SlotView(done: done, preprocess: preprocess, progress: progress)[
            keyPath: keyPath]
        lock.unlock()
        handler?(payload)
    }

    fileprivate struct SlotView {
        let done: (@Sendable (String) -> Void)?
        let preprocess: (@Sendable (String) -> Void)?
        let progress: (@Sendable (String) -> Void)?
    }
}

/// Copies the kernel-allocated C string into a Swift String and frees it.
/// (the kernel mallocs each payload and never frees it — the original Node bindings leaked
/// every payload; we don't have to.)
private func consume(_ cstr: UnsafeMutablePointer<CChar>?) -> String {
    guard let cstr else { return "" }
    defer { free(cstr) }
    let payload = String(cString: cstr)
    GoldenDump.record(payload)
    return payload
}

/// M0 golden-fixture capture: when `KEEL_DUMP_DIR` is set, every callback
/// payload is written verbatim to a numbered file. These fixtures are the
/// byte-level contract the Rust kernel (keel) must reproduce.
enum GoldenDump {
    private static let dir: URL? = ProcessInfo.processInfo
        .environment["KEEL_DUMP_DIR"]
        .map { URL(fileURLWithPath: $0, isDirectory: true) }
    nonisolated(unsafe) private static var counter = 0
    private static let lock = NSLock()

    static func record(_ payload: String) {
        guard let dir else { return }
        lock.lock()
        defer { lock.unlock() }
        counter += 1
        try? payload.write(
            to: dir.appendingPathComponent(String(format: "%04d.json", counter)),
            atomically: true, encoding: .utf8)
    }
}

private let doneTrampoline: KeelCCallback = { cstr in
    KeelCallbackSlots.fire(\.done, consume(cstr))
}

private let preprocessTrampoline: KeelCCallback = { cstr in
    KeelCallbackSlots.fire(\.preprocess, consume(cstr))
}

private let progressTrampoline: KeelCCallback = { cstr in
    KeelCallbackSlots.fire(\.progress, consume(cstr))
}

// MARK: - Library

/// dlopen/dlsym wrapper around keel.dylib — the pure-Rust MTP kernel.
///
/// keel is self-contained (nusb links statically; no libusb sibling), so the
/// single dylib is all that ships. Kept the `KeelLibrary` type name and the
/// frozen C-ABI symbol surface. keel is a conformance-proven byte-exact drop-in for the retired
/// kernel, so the Swift side is unchanged.
public final class KeelLibrary: Sendable {
    static let dylibName = "keel.dylib"

    public enum LoadError: Error, CustomStringConvertible {
        case libraryNotFound(String)
        case dlopenFailed(String, String)
        case symbolMissing(String)

        public var description: String {
            switch self {
            case .libraryNotFound(let path):
                return "\(KeelLibrary.dylibName) not found at \(path)"
            case .dlopenFailed(let path, let why):
                return "dlopen(\(path)) failed: \(why)"
            case .symbolMissing(let name):
                return "symbol \(name) missing from \(KeelLibrary.dylibName)"
            }
        }
    }

    private struct Symbols: @unchecked Sendable {
        let initialize: FnCB
        let fetchDeviceInfo: FnCB
        let fetchStorages: FnCB
        let fileExists: FnJSONCB
        let deleteFile: FnJSONCB
        let makeDirectory: FnJSONCB
        let renameFile: FnJSONCB
        let walk: FnJSONCB
        let uploadFiles: FnJSON3CB
        let downloadFiles: FnJSON3CB
        let dispose: FnCB
        /// Ferry extensions to the kernel — optional so an older dylib without
        /// them still loads (the core 11 + Dispose are required).
        let cancelTransfer: FnVoid?
        let fetchThumbnail: FnJSONCB?
        let walkStream: FnJSON2CB?
    }

    private let symbols: Symbols
    public let libraryPath: String

    /// True when the loaded kernel supports mid-flight cancellation.
    public var supportsCancellation: Bool { symbols.cancelTransfer != nil }

    /// True when the loaded kernel can fetch device thumbnails.
    public var supportsThumbnails: Bool { symbols.fetchThumbnail != nil }

    /// True when the loaded kernel can stream a walk in batches.
    public var supportsWalkStream: Bool { symbols.walkStream != nil }

    public init(directory: URL) throws {
        let path = directory.appendingPathComponent(Self.dylibName).path
        guard FileManager.default.fileExists(atPath: path) else {
            throw LoadError.libraryNotFound(path)
        }
        guard let handle = dlopen(path, RTLD_NOW | RTLD_LOCAL) else {
            throw LoadError.dlopenFailed(path, String(cString: dlerror()))
        }

        func sym<T>(_ name: String, as type: T.Type) throws -> T {
            guard let raw = dlsym(handle, name) else {
                throw LoadError.symbolMissing(name)
            }
            return unsafeBitCast(raw, to: T.self)
        }

        self.libraryPath = path
        self.symbols = Symbols(
            initialize: try sym("Initialize", as: FnCB.self),
            fetchDeviceInfo: try sym("FetchDeviceInfo", as: FnCB.self),
            fetchStorages: try sym("FetchStorages", as: FnCB.self),
            fileExists: try sym("FileExists", as: FnJSONCB.self),
            deleteFile: try sym("DeleteFile", as: FnJSONCB.self),
            makeDirectory: try sym("MakeDirectory", as: FnJSONCB.self),
            renameFile: try sym("RenameFile", as: FnJSONCB.self),
            walk: try sym("Walk", as: FnJSONCB.self),
            uploadFiles: try sym("UploadFiles", as: FnJSON3CB.self),
            downloadFiles: try sym("DownloadFiles", as: FnJSON3CB.self),
            dispose: try sym("Dispose", as: FnCB.self),
            cancelTransfer: dlsym(handle, "CancelTransfer")
                .map { unsafeBitCast($0, to: FnVoid.self) },
            fetchThumbnail: dlsym(handle, "FetchThumbnail")
                .map { unsafeBitCast($0, to: FnJSONCB.self) },
            walkStream: dlsym(handle, "WalkStream")
                .map { unsafeBitCast($0, to: FnJSON2CB.self) }
        )
    }

    /// Flags the in-flight transfer for cancellation. Non-blocking (just an
    /// atomic store in the kernel), so it deliberately bypasses the FFI
    /// queue — the queue is busy running the very transfer being cancelled.
    /// No-op on unpatched kernels.
    func rawCancelTransfer() {
        symbols.cancelTransfer?()
    }

    /// Locates the dylib directory: `KEEL_LIB_DIR` env override, the app
    /// bundle's `Resources/bin`, then `Libraries/<arch>` relative to cwd
    /// (the dev/CLI case).
    public static func defaultLibraryDirectory() -> URL {
        if let env = ProcessInfo.processInfo.environment["KEEL_LIB_DIR"] {
            return URL(fileURLWithPath: env, isDirectory: true)
        }
        let bundled = Bundle.main.resourceURL?.appendingPathComponent("bin")
        if let bundled,
            FileManager.default.fileExists(
                atPath: bundled.appendingPathComponent(dylibName).path)
        {
            return bundled
        }
        #if arch(arm64)
            let arch = "arm64"
        #else
            let arch = "amd64"
        #endif
        return URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
            .appendingPathComponent("Libraries/\(arch)", isDirectory: true)
    }

    // MARK: Raw blocking calls (installed slots must already be set)

    func rawCall(_ fn: KeelSimpleFunction) {
        switch fn {
        case .initialize: symbols.initialize(doneTrampoline)
        case .fetchDeviceInfo: symbols.fetchDeviceInfo(doneTrampoline)
        case .fetchStorages: symbols.fetchStorages(doneTrampoline)
        case .dispose: symbols.dispose(doneTrampoline)
        }
    }

    func rawCall(_ fn: KeelJSONFunction, json: String) {
        json.withCString { cstr in
            let mutable = UnsafeMutablePointer(mutating: cstr)
            switch fn {
            case .fileExists: symbols.fileExists(mutable, doneTrampoline)
            case .deleteFile: symbols.deleteFile(mutable, doneTrampoline)
            case .makeDirectory: symbols.makeDirectory(mutable, doneTrampoline)
            case .renameFile: symbols.renameFile(mutable, doneTrampoline)
            case .walk: symbols.walk(mutable, doneTrampoline)
            }
        }
    }

    func rawTransfer(_ direction: KeelTransferDirection, json: String) {
        json.withCString { cstr in
            let mutable = UnsafeMutablePointer(mutating: cstr)
            switch direction {
            case .download:
                symbols.downloadFiles(
                    mutable, preprocessTrampoline, progressTrampoline, doneTrampoline)
            case .upload:
                symbols.uploadFiles(
                    mutable, preprocessTrampoline, progressTrampoline, doneTrampoline)
            }
        }
    }

    /// Calls the optional FetchThumbnail export. Returns false (firing nothing)
    /// when the loaded kernel lacks it, so the caller can fall back.
    func rawFetchThumbnail(json: String) -> Bool {
        guard let fn = symbols.fetchThumbnail else { return false }
        json.withCString { cstr in
            fn(UnsafeMutablePointer(mutating: cstr), doneTrampoline)
        }
        return true
    }

    /// Calls the optional WalkStream export. The batch callback rides the
    /// progress slot (each fires a JSON array of entries); the done callback the
    /// done slot. Returns false when the kernel lacks the export.
    func rawWalkStream(json: String) -> Bool {
        guard let fn = symbols.walkStream else { return false }
        json.withCString { cstr in
            fn(UnsafeMutablePointer(mutating: cstr), progressTrampoline, doneTrampoline)
        }
        return true
    }
}

enum KeelSimpleFunction {
    case initialize, fetchDeviceInfo, fetchStorages, dispose
}

enum KeelJSONFunction {
    case fileExists, deleteFile, makeDirectory, renameFile, walk
}

public enum KeelTransferDirection: String, Sendable {
    case download  // phone -> Mac
    case upload  // Mac -> phone
}
