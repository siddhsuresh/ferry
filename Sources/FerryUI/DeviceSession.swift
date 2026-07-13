import AppKit
import Foundation
import KeelKit
import Observation
import UniformTypeIdentifiers

// MARK: - File categories (wireframe 3c/3f filter chips)

public enum FileCategory: String, CaseIterable, Identifiable, Sendable {
    case all = "All"
    case images = "Images"
    case video = "Video"
    case audio = "Audio"
    case documents = "Documents"
    case archives = "Archives"

    public var id: String { rawValue }

    static let imageExts: Set<String> = ["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tiff", "dng", "raw"]
    static let videoExts: Set<String> = ["mp4", "mkv", "mov", "webm", "avi", "3gp", "m4v", "ts"]
    static let audioExts: Set<String> = ["mp3", "flac", "ogg", "m4a", "wav", "aac", "opus", "amr"]
    static let documentExts: Set<String> = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "epub", "rtf", "pages", "numbers", "key"]
    static let archiveExts: Set<String> = ["zip", "rar", "7z", "gz", "tar", "bz2", "xz", "apk"]

    public func matches(_ file: KeelFile) -> Bool {
        if self == .all { return true }
        if file.isFolder { return false }
        let ext = file.extension.trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased()
        switch self {
        case .all: return true
        case .images: return Self.imageExts.contains(ext)
        case .video: return Self.videoExts.contains(ext)
        case .audio: return Self.audioExts.contains(ext)
        case .documents: return Self.documentExts.contains(ext)
        case .archives: return Self.archiveExts.contains(ext)
        }
    }
}

// MARK: - Shelf items (wireframe 3c: gather from phone AND Finder, send once)

public enum ShelfItem: Identifiable, Hashable, Sendable {
    /// A file/folder on the phone, destined for the Mac.
    case phone(KeelFile)
    /// A file/folder on the Mac (Finder drop), destined for the phone.
    case local(URL)

    public var id: String {
        switch self {
        case .phone(let f): return "phone:\(f.path)"
        case .local(let u): return "local:\(u.path)"
        }
    }

    public var name: String {
        switch self {
        case .phone(let f): return f.name
        case .local(let u): return u.lastPathComponent
        }
    }

    public var byteSize: Int64 {
        switch self {
        case .phone(let f): return f.size
        case .local(let u):
            return (try? u.resourceValues(forKeys: [.fileSizeKey]).fileSize)
                .map(Int64.init) ?? 0
        }
    }
}

// MARK: - Settings (wireframe 3m — tiny on purpose)

@MainActor
@Observable
public final class FerrySettings {
    public static let shared = FerrySettings()

    /// "Save phone files to:"
    public var destinationPath: String {
        didSet { UserDefaults.standard.set(destinationPath, forKey: "destinationPath") }
    }
    /// "Play a sound when a transfer finishes"
    public var playSound: Bool {
        didSet { UserDefaults.standard.set(playSound, forKey: "playSound") }
    }
    /// "Open Finder when done"
    public var revealInFinder: Bool {
        didSet { UserDefaults.standard.set(revealInFinder, forKey: "revealInFinder") }
    }
    /// "Skip files that already exist on the Mac"
    public var skipExisting: Bool {
        didSet { UserDefaults.standard.set(skipExisting, forKey: "skipExisting") }
    }

    public var destinationURL: URL {
        URL(fileURLWithPath: (destinationPath as NSString).expandingTildeInPath,
            isDirectory: true)
    }

    private init() {
        let d = UserDefaults.standard
        destinationPath = d.string(forKey: "destinationPath") ?? "~/Downloads/From phone"
        playSound = d.object(forKey: "playSound") as? Bool ?? true
        revealInFinder = d.object(forKey: "revealInFinder") as? Bool ?? true
        skipExisting = d.object(forKey: "skipExisting") as? Bool ?? false
    }
}

// MARK: - Session

/// UI-facing state machine over `KeelEngine`, redesigned around the Shelf
/// (design doc turn 3): browse and gather anywhere, then send once.
///
/// MTP is a single-session serial protocol — jobs queue and run back-to-back;
/// each kernel call batches all its sources into one streamed session.
@MainActor
@Observable
public final class DeviceSession {
    public enum Phase: Equatable {
        /// 3a — waiting for a phone, auto-detecting.
        case idle
        /// 3b — phone found, reading storage. Payload is the device name.
        case handshake(String)
        /// 3c — browsing.
        case connected
        case failed(String)
    }

    public struct ActiveTransfer: Equatable {
        public var direction: KeelTransferDirection
        public var currentFile: String = ""
        public var filesSent: Int64 = 0
        public var totalFiles: Int64 = 0
        public var progress: Float = 0
        public var speedMBps: Double = 0
        public var bytesSent: Int64 = 0
        public var bytesTotal: Int64 = 0
        /// Becomes true when the first sampled progress payload arrives. Fast
        /// transfers may begin with only a preparing state before the kernel's
        /// progress sampler has emitted its first event.
        public var hasProgress = false

        /// 3h "2 min left" — derived from bulk bytes remaining and speed.
        public var etaSeconds: Int? {
            guard speedMBps > 0.1, bytesTotal > bytesSent else { return nil }
            return Int(Double(bytesTotal - bytesSent) / (speedMBps * 1_000_000))
        }

        public var title: String {
            direction == .download ? "Sending to Mac" : "Sending to phone"
        }
    }

    public struct TransferJob: Identifiable, Equatable {
        public let id = UUID()
        public let direction: KeelTransferDirection
        public let sources: [String]
        public let destination: String
        public let revealTarget: URL?
        /// Silent jobs (preview fetches) skip the done card and chime.
        public var silent: Bool = false

        /// 3h "Next in queue" line, e.g. `"Music for dad" → phone (2.1 GB)`.
        public var queueLabel: String {
            let what = sources.count == 1
                ? "“\((sources[0] as NSString).lastPathComponent)”"
                : "\(sources.count) items"
            return direction == .download ? "\(what) → Mac" : "\(what) → phone"
        }
    }

    /// 3i — quiet confirmation shown in the shelf bar.
    public struct Completion: Equatable {
        public let filesCopied: Int64
        public let direction: KeelTransferDirection
        public let revealURL: URL?

        public var headline: String {
            let dest = direction == .download ? "your Mac" : "the phone"
            return "\(filesCopied) file\(filesCopied == 1 ? "" : "s") copied to \(dest)"
        }
    }

    /// 3j — phone unplugged mid-transfer.
    public struct Interruption: Equatable {
        public let made: Int64
        public let of: Int64
    }

    // Connection
    public private(set) var phase: Phase = .idle
    public private(set) var device: KeelDeviceInfo?
    public private(set) var storages: [KeelStorage] = []
    public var selectedStorageID: UInt32? {
        // Only auto-navigate on a *user* storage switch (already connected).
        // During connect() the phase is .handshake and connect() drives the
        // initial walk itself — navigating here too would run two FFI ops at
        // once and hang. See KeelEngine's FFI gate.
        didSet {
            if oldValue != selectedStorageID, phase == .connected {
                Task { await navigateToRoot() }
            }
        }
    }

    // Browsing
    public private(set) var path: String = "/"
    public private(set) var files: [KeelFile] = []
    public private(set) var isListing = false
    public var category: FileCategory = .all
    public var viewMode: ViewMode = .grid
    public enum ViewMode: String { case grid, list }

    // Shelf
    public private(set) var shelf: [ShelfItem] = []

    // Transfers
    public private(set) var transfer: ActiveTransfer?
    public private(set) var queue: [TransferJob] = []
    public private(set) var completion: Completion?
    public var interruption: Interruption?

    // Search (3f)
    public var isSearching = false
    public var searchQuery = ""
    public var searchCategory: FileCategory = .all
    public private(set) var searchIndex: [KeelFile] = []
    public private(set) var isIndexing = false

    // Preview (3g)
    public var previewURL: URL?
    public private(set) var isPreparingPreview = false

    public private(set) var lastError: String?

    private var engine: KeelEngine?
    private var isAttemptingConnect = false
    private var indexedStorageID: UInt32?

    // Thumbnails: objectId → loaded image, so a tile that scrolls back into
    // view doesn't re-fetch over USB. Cleared on disconnect.
    private var thumbnailCache: [UInt32: NSImage] = [:]

    public init() {}

    /// Device thumbnail for a file (cached). Returns nil for folders, non-image
    /// types, or when the kernel/device has none — the tile keeps its glyph.
    public func thumbnailImage(for file: KeelFile) async -> NSImage? {
        if let cached = thumbnailCache[file.objectId] { return cached }
        guard let engine, let sid = selectedStorageID, !file.isFolder else { return nil }
        guard let data = await engine.thumbnail(storageId: sid, path: file.path),
            let image = NSImage(data: data)
        else { return nil }
        thumbnailCache[file.objectId] = image
        return image
    }

    public var settings: FerrySettings { FerrySettings.shared }

    public var selectedStorage: KeelStorage? {
        storages.first { $0.sid == selectedStorageID }
    }

    public var pathComponents: [String] {
        path.split(separator: "/").map(String.init)
    }

    /// Files currently visible: category filter applied, folders first.
    public var visibleFiles: [KeelFile] {
        let filtered = files.filter { category.matches($0) || ($0.isFolder && category == .all) }
        return filtered.filter(\.isFolder) + filtered.filter { !$0.isFolder }
    }

    public var searchResults: [KeelFile] {
        guard !searchQuery.isEmpty else { return [] }
        return searchIndex.filter {
            $0.name.localizedCaseInsensitiveContains(searchQuery)
                && (searchCategory == .all || searchCategory.matches($0))
        }
    }

    public func searchCount(for category: FileCategory) -> Int {
        guard !searchQuery.isEmpty else { return 0 }
        return searchIndex.filter {
            $0.name.localizedCaseInsensitiveContains(searchQuery)
                && (category == .all || category.matches($0))
        }.count
    }

    // MARK: - Lifecycle (3a → 3b → 3c)

    /// Auto-detection only — the design has no Connect button (3a). Silent
    /// attempts don't surface errors, so the start screen never flickers.
    public func connect(silent: Bool = true) async {
        guard !isAttemptingConnect, phase == .idle else { return }
        isAttemptingConnect = true
        defer { isAttemptingConnect = false }

        do {
            let engine = try self.engine ?? KeelEngine()
            self.engine = engine
            let info = try await initializeWithRetry(engine)
            device = info
            // 3b: "<device> found · Reading storage…"
            phase = .handshake(info.displayName)
            storages = try await engine.storages()
            selectedStorageID = storages.first?.sid
            await navigateToRoot()
            phase = .connected
            lastError = nil
        } catch let error as KeelError where error.isDeviceNotFound {
            phase = .idle
        } catch let error as KeelError where error.isDeviceSetupFailure {
            phase = .idle
            lastError = Self.setupFailureMessage()
        } catch {
            phase = silent ? .idle : .failed("\(error)")
        }
    }

    public func autoConnectLoop() async {
        while !Task.isCancelled {
            if phase == .idle {
                await connect()
            }
            try? await Task.sleep(for: .seconds(3))
        }
    }

    public func disconnect() async {
        await engine?.dispose()
        phase = .idle
        device = nil
        storages = []
        selectedStorageID = nil
        files = []
        path = "/"
        transfer = nil
        queue = []
        shelf = []
        completion = nil
        searchIndex = []
        thumbnailCache = [:]
        indexedStorageID = nil
        isSearching = false
    }

    private func initializeWithRetry(
        _ engine: KeelEngine, attempts: Int = 3
    ) async throws -> KeelDeviceInfo {
        var lastFailure: Error = KeelError.notConnected
        for attempt in 1...attempts {
            do {
                return try await engine.initialize()
            } catch let error as KeelError where error.isDeviceSetupFailure {
                lastFailure = error
                await engine.dispose()
                if attempt < attempts {
                    try? await Task.sleep(for: .seconds(1.5))
                }
            }
        }
        throw lastFailure
    }

    private static func competingMTPApps() -> [String] {
        let suspects = [
            "Android File Transfer", "Android File Transfer Agent",
            "Smart Switch", "Samsung Smart Switch", "Kies",
        ]
        return NSWorkspace.shared.runningApplications.compactMap { app in
            guard let name = app.localizedName else { return nil }
            return suspects.contains(where: { name.localizedCaseInsensitiveContains($0) })
                ? name : nil
        }
    }

    private static func setupFailureMessage() -> String {
        let blockers = competingMTPApps()
        if !blockers.isEmpty {
            return "Quit \(blockers.joined(separator: " and ")) — it's holding the phone. "
                + "Ferry will connect automatically once it's closed."
        }
        return "USB setup failed — unlock the phone, re-select File transfer mode, "
            + "or re-plug the cable. Retrying automatically…"
    }

    // MARK: - Navigation

    public func navigateToRoot() async { await navigate(to: "/") }

    public func navigate(to newPath: String) async {
        guard let engine, let sid = selectedStorageID else { return }
        isListing = true
        defer { isListing = false }
        do {
            files = try await engine.list(storageId: sid, path: newPath)
            path = newPath
            lastError = nil
        } catch let error as KeelError where error.isDeviceNotFound {
            phase = .idle
            lastError = "Phone disconnected."
        } catch {
            lastError = "\(error)"
        }
    }

    public func open(_ file: KeelFile) async {
        guard file.isFolder else { return }
        await navigate(to: file.path)
    }

    public func navigateUp() async {
        guard path != "/" else { return }
        let parent = (path as NSString).deletingLastPathComponent
        await navigate(to: parent.isEmpty ? "/" : parent)
    }

    public func refresh() async { await navigate(to: path) }

    // MARK: - Mutation (3e)

    public func makeFolder(named name: String) async {
        guard let engine, let sid = selectedStorageID else { return }
        let newPath = path == "/" ? "/\(name)" : "\(path)/\(name)"
        do {
            try await engine.makeDirectory(storageId: sid, path: newPath)
            await refresh()
        } catch { lastError = "\(error)" }
    }

    public func rename(_ file: KeelFile, to newName: String) async {
        guard let engine, let sid = selectedStorageID else { return }
        do {
            try await engine.rename(storageId: sid, path: file.path, to: newName)
            await refresh()
        } catch { lastError = "\(error)" }
    }

    public func delete(_ selection: [KeelFile]) async {
        guard let engine, let sid = selectedStorageID, !selection.isEmpty else { return }
        do {
            try await engine.delete(storageId: sid, paths: selection.map(\.path))
            shelf.removeAll { item in
                if case .phone(let f) = item { return selection.contains(f) }
                return false
            }
            await refresh()
        } catch { lastError = "\(error)" }
    }

    // MARK: - Shelf

    public var shelfTotalBytes: Int64 { shelf.reduce(0) { $0 + $1.byteSize } }
    public var shelfPhoneFiles: [KeelFile] {
        shelf.compactMap { if case .phone(let f) = $0 { return f }; return nil }
    }
    public var shelfLocalURLs: [URL] {
        shelf.compactMap { if case .local(let u) = $0 { return u }; return nil }
    }

    public func addToShelf(_ files: [KeelFile]) {
        for f in files {
            let item = ShelfItem.phone(f)
            if !shelf.contains(item) { shelf.append(item) }
        }
        completion = nil
    }

    public func addToShelf(_ urls: [URL]) {
        for u in urls {
            let item = ShelfItem.local(u)
            if !shelf.contains(item) { shelf.append(item) }
        }
        completion = nil
    }

    public func removeFromShelf(_ item: ShelfItem) {
        shelf.removeAll { $0 == item }
    }

    public func clearShelf() { shelf = [] }

    /// 3c "Send to Mac" — ships the shelf's phone items to a folder the user
    /// picks each time (no preset destination). `dest` comes from the picker.
    public func sendShelfToMac(to dest: URL) {
        var sources = shelfPhoneFiles
        guard !sources.isEmpty else { return }

        // 3m "Skip files that already exist on the Mac" — top-level files
        // only; folder contents are streamed by the kernel and can't be
        // filtered per-child from here.
        if settings.skipExisting {
            sources = sources.filter { file in
                file.isFolder
                    || !FileManager.default.fileExists(
                        atPath: dest.appendingPathComponent(file.name).path)
            }
            guard !sources.isEmpty else {
                completion = Completion(filesCopied: 0, direction: .download, revealURL: nil)
                shelf.removeAll { if case .phone = $0 { return true }; return false }
                return
            }
        }

        // Remember the choice as the picker's next starting point.
        settings.destinationPath = dest.path.replacingOccurrences(
            of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~")
        try? FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        shelf.removeAll { if case .phone = $0 { return true }; return false }
        enqueue(TransferJob(
            direction: .download,
            sources: sources.map(\.path),
            destination: dest.path,
            revealTarget: dest))
    }

    /// 3c "Send to phone" — ships the shelf's Finder items into the folder
    /// currently on screen.
    public func sendShelfToPhone() {
        let urls = shelfLocalURLs
        guard !urls.isEmpty else { return }
        shelf.removeAll { if case .local = $0 { return true }; return false }
        enqueue(TransferJob(
            direction: .upload,
            sources: urls.map(\.path),
            destination: path,
            revealTarget: nil))
    }

    /// 3e "Send to Mac now…" — bypass the shelf for an immediate copy to a
    /// user-chosen folder (no preset destination).
    public func sendToMacNow(_ files: [KeelFile], to dest: URL) {
        guard !files.isEmpty else { return }
        settings.destinationPath = dest.path.replacingOccurrences(
            of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~")
        try? FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        enqueue(TransferJob(
            direction: .download,
            sources: files.map(\.path),
            destination: dest.path,
            revealTarget: dest))
    }

    /// 3n "Drop files from Finder to copy them here" — direct upload.
    public func uploadHere(_ urls: [URL]) {
        guard !urls.isEmpty else { return }
        enqueue(TransferJob(
            direction: .upload,
            sources: urls.map(\.path),
            destination: path,
            revealTarget: nil))
    }

    // MARK: - Search (3f)

    public func enterSearch() {
        isSearching = true
        completion = nil
        if indexedStorageID != selectedStorageID {
            Task { await buildSearchIndex() }
        }
    }

    public func exitSearch() {
        isSearching = false
        searchQuery = ""
        searchCategory = .all
    }

    public func addAllResultsToShelf() {
        addToShelf(searchResults.filter { !$0.isFolder })
    }

    private func buildSearchIndex() async {
        guard let engine, let sid = selectedStorageID, !isIndexing else { return }
        isIndexing = true
        defer { isIndexing = false }
        do {
            searchIndex = try await engine.list(storageId: sid, path: "/", recursive: true)
            indexedStorageID = sid
        } catch {
            lastError = "Search indexing failed: \(error)"
        }
    }

    // MARK: - Preview (3g)

    /// Space bar / double-click: copies the file to a temp folder over USB,
    /// then Quick Look takes over (images zoom, videos scrub, audio plays).
    public func preview(_ file: KeelFile) {
        guard !file.isFolder else { return }
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ferry-previews", isDirectory: true)
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        let localURL = tempDir.appendingPathComponent(file.name)

        if FileManager.default.fileExists(atPath: localURL.path) {
            previewURL = localURL
            return
        }
        isPreparingPreview = true
        enqueue(TransferJob(
            direction: .download,
            sources: [file.path],
            destination: tempDir.path,
            revealTarget: localURL,
            silent: true))
    }

    // MARK: - Transfer queue (3h / 3i / 3j)

    public func cancelCurrentTransfer() {
        guard transfer != nil, let engine else { return }
        Task { await engine.cancelActiveTransfer() }
    }

    public func cancelAllTransfers() {
        queue.removeAll()
        cancelCurrentTransfer()
    }

    public func dismissCompletion() { completion = nil }

    private func enqueue(_ job: TransferJob) {
        queue.append(job)
        completion = nil
        if transfer == nil {
            Task { await pump() }
        }
    }

    private func pump() async {
        var movedFiles: Int64 = 0
        var lastDirection: KeelTransferDirection = .download
        var lastReveal: URL?
        var sawCancellation = false
        var sawFailure = false
        var allSilent = true

        while !queue.isEmpty {
            let job = queue.removeFirst()
            let outcome = await run(job)
            switch outcome {
            case .completed(let files):
                if job.silent {
                    // Preview fetch: hand the file to Quick Look.
                    isPreparingPreview = false
                    if let target = job.revealTarget {
                        previewURL = target
                    }
                } else {
                    allSilent = false
                    movedFiles += files
                    lastDirection = job.direction
                    lastReveal = job.revealTarget
                }
            case .cancelled:
                sawCancellation = true
                isPreparingPreview = false
            case .failed:
                sawFailure = true
                allSilent = allSilent && job.silent
                isPreparingPreview = false
            }
        }

        guard !allSilent else { return }

        // 3i — quiet confirmation, chime, optional Finder reveal.
        if !sawCancellation && !sawFailure {
            completion = Completion(
                filesCopied: movedFiles, direction: lastDirection, revealURL: lastReveal)
            if settings.playSound {
                NSSound(named: "Glass")?.play()
            }
            if settings.revealInFinder, let reveal = lastReveal {
                NSWorkspace.shared.activateFileViewerSelecting([reveal])
            }
        }
        if NSApp?.isActive == false {
            NSApp?.requestUserAttention(.informationalRequest)
        }
    }

    private enum JobOutcome {
        case completed(files: Int64)
        case cancelled
        case failed
    }

    private func run(_ job: TransferJob) async -> JobOutcome {
        guard let engine, let sid = selectedStorageID else { return .failed }
        let startedAt = Date()
        transfer = ActiveTransfer(direction: job.direction)
        defer { transfer = nil }
        // Let SwiftUI paint the tray before starting the blocking FFI work.
        // Without this yield, a short transfer can set and clear `transfer`
        // during one main-actor turn and the tray is never visible.
        try? await Task.sleep(for: .milliseconds(40))
        var lastFilesSent: Int64 = 0
        var lastTotal: Int64 = 0
        do {
            let stream = await engine.transfer(
                job.direction, storageId: sid, sources: job.sources,
                destination: job.destination)
            for try await event in stream {
                switch event {
                case .preprocess(let info):
                    transfer?.currentFile = info.name
                case .progress(let info):
                    transfer?.currentFile = info.name
                    transfer?.filesSent = info.filesSent
                    transfer?.totalFiles = info.totalFiles
                    transfer?.progress = info.bulkFileSize.progress
                    transfer?.speedMBps = info.speed
                    transfer?.bytesSent = info.bulkFileSize.sent
                    transfer?.bytesTotal = info.bulkFileSize.total
                    transfer?.hasProgress = true
                    lastFilesSent = info.filesSent
                    lastTotal = info.totalFiles
                case .completed:
                    break
                }
            }
            if !job.silent {
                // Keep the tray visible long enough to be useful even when a
                // small transfer completes before the first sampler tick.
                let remaining = 0.25 - Date().timeIntervalSince(startedAt)
                if remaining > 0 {
                    try? await Task.sleep(for: .milliseconds(Int(remaining * 1_000)))
                }
                await refresh()
            }
            // `filesSent` is the number actually completed. Do not fall back
            // to `totalFiles`, which would turn a partial/missing final
            // progress event into an over-count.
            return .completed(files: lastFilesSent)
        } catch let error as KeelError where error.isCancellation {
            await refresh()
            return .cancelled
        } catch let error as KeelError where error.isDeviceNotFound {
            // 3j — "Phone unplugged. N of M files made it."
            interruption = Interruption(made: lastFilesSent, of: lastTotal)
            queue.removeAll()
            phase = .idle
            return .failed
        } catch {
            if !job.silent { lastError = "\(job.queueLabel): \(error)" }
            return .failed
        }
    }
}
