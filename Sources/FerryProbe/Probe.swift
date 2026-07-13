import Foundation
import KeelKit

// ferry-probe — proves the Swift ↔ keel.dylib bridge end-to-end:
// initialize a session, print device info + storages, list the storage root,
// dispose. Run with a phone plugged in and USB mode set to "File Transfer".
//
//   swift run ferry-probe [--lib-dir <dir>] [--path </some/phone/path>]

@main
enum Probe {
    static func fail(_ message: String) -> Never {
        FileHandle.standardError.write(Data("error: \(message)\n".utf8))
        exit(1)
    }

    static func format(bytes: Int64?) -> String {
        guard let bytes else { return "?" }
        return ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    static func main() async {
        var libDir: URL?
        var walkPath = "/"
        var golden = false
        var args = Array(CommandLine.arguments.dropFirst())
        while let arg = args.first {
            args.removeFirst()
            switch arg {
            case "--lib-dir":
                guard let value = args.first else { fail("--lib-dir needs a value") }
                args.removeFirst()
                libDir = URL(fileURLWithPath: value, isDirectory: true)
            case "--path":
                guard let value = args.first else { fail("--path needs a value") }
                args.removeFirst()
                walkPath = value
            case "--golden":
                golden = true
            case "--concurrent":
                break  // handled after engine init
            default:
                fail("unknown argument \(arg)")
            }
        }

        if golden {
            await runGoldenSession(libDir: libDir)
            return
        }

        let engine: KeelEngine
        do {
            engine = try KeelEngine(libraryDirectory: libDir)
        } catch {
            fail("\(error)")
        }

        if CommandLine.arguments.contains("--concurrent") {
            await runConcurrentStress(engine)
            return
        }

        print("ferry-probe — loaded \(await engine.libraryPath)")

        // 1. Session
        let device: KeelDeviceInfo
        do {
            device = try await engine.initialize()
        } catch let error as KeelError where error.isDeviceNotFound {
            print(
                """

                ✗ no MTP device found (\(error))

                  1. plug the phone in over USB
                  2. on the phone, tap the USB notification and choose
                     "File Transfer / Android Auto" (MTP)
                  3. quit anything else holding the device (Android File Transfer,
                     Android File Transfer, adb)
                  4. re-run ferry-probe
                """)
            exit(2)
        } catch {
            fail("initialize failed: \(error)")
        }

        let mtp = device.mtpDeviceInfo
        print(
            """

            ✓ device connected
              name:      \(device.displayName)
              version:   \(mtp?.deviceVersion ?? "?")
              serial:    \(mtp?.serialNumber ?? "?")
            """)

        // 2. Storages
        let storages: [KeelStorage]
        do {
            storages = try await engine.storages()
        } catch {
            fail("fetchStorages failed: \(error)")
        }

        print("\n✓ storages (\(storages.count))")
        for storage in storages {
            print(
                "  [\(storage.sid)] \(storage.description)"
                    + "  free \(format(bytes: storage.freeSpace))"
                    + " / \(format(bytes: storage.maxCapacity))")
        }

        // 3. Walk
        if let first = storages.first {
            do {
                let files = try await engine.list(storageId: first.sid, path: walkPath)
                print("\n✓ \(walkPath) on \(first.description) — \(files.count) entries")
                for file in files.prefix(15) {
                    let marker = file.isFolder ? "▸" : " "
                    let size = file.isFolder ? "" : "  (\(format(bytes: file.size)))"
                    print("  \(marker) \(file.name)\(size)")
                }
                if files.count > 15 { print("  … \(files.count - 15) more") }
            } catch {
                print("\n✗ walk \(walkPath) failed: \(error)")
            }
        }

        await engine.dispose()
        print("\n✓ session disposed — bridge verified")
    }

    /// Reproduces the app's connect-time race: after initialize, fire many
    /// engine calls CONCURRENTLY (as the storage-switch didSet did against
    /// connect()'s walk). Pre-fix this hangs on the shared callback slots;
    /// with the FFI gate every call must return.
    static func runConcurrentStress(_ engine: KeelEngine) async {
        do {
            _ = try await engine.initialize()
            let storages = try await engine.storages()
            guard let sid = storages.first?.sid else { fail("no storage") }
            print("initialized — firing 12 concurrent ops (would hang pre-fix)…")

            try await withThrowingTaskGroup(of: Int.self) { group in
                for i in 0..<12 {
                    group.addTask {
                        // Mix of storages + walks, all racing.
                        if i % 2 == 0 {
                            _ = try await engine.storages()
                        } else {
                            _ = try await engine.list(storageId: sid, path: "/")
                        }
                        return i
                    }
                }
                var done = 0
                for try await _ in group { done += 1; print("  ✓ op \(done)/12 returned") }
            }
            await engine.dispose()
            print("✓ all 12 concurrent ops returned — no hang, slots intact")
        } catch {
            fail("concurrent stress failed: \(error)")
        }
    }

    /// M0 golden capture: a scripted session exercising every keel export,
    /// including transfer progress callbacks. Run with KEEL_DUMP_DIR set so
    /// KeelKit writes each raw payload to disk. Creates (and removes) a
    /// `keel-golden-test` folder under /Download on the phone.
    static func runGoldenSession(libDir: URL?) async {
        let engine: KeelEngine
        do {
            engine = try KeelEngine(libraryDirectory: libDir)
        } catch {
            fail("\(error)")
        }
        print("golden session — \(await engine.libraryPath)")

        func step(_ name: String, _ body: () async throws -> Void) async {
            do {
                try await body()
                print("  ✓ \(name)")
            } catch {
                print("  ✗ \(name): \(error)")
            }
        }

        // 1-2. Initialize + FetchDeviceInfo
        var sid: UInt32 = 0
        await step("Initialize") { _ = try await engine.initialize() }
        await step("FetchDeviceInfo") { _ = try await engine.deviceInfo() }
        // 3. FetchStorages
        await step("FetchStorages") {
            let storages = try await engine.storages()
            sid = storages.first?.sid ?? 0
        }
        guard sid != 0 else { fail("no storage — is the phone unlocked?") }

        let base = "/Download/keel-golden-test"
        // 4. Walk (root, non-recursive) + a nested walk
        await step("Walk /") { _ = try await engine.list(storageId: sid, path: "/") }
        await step("Walk /Download") {
            _ = try await engine.list(storageId: sid, path: "/Download")
        }
        // 5. MakeDirectory (fresh + idempotent repeat)
        await step("MakeDirectory") {
            try await engine.makeDirectory(storageId: sid, path: base)
            try await engine.makeDirectory(storageId: sid, path: base)
        }
        // 6. FileExists (hit + miss)
        await step("FileExists") {
            _ = try await engine.filesExist(
                storageId: sid, paths: [base, "\(base)/definitely-missing.bin"])
        }
        // 7. UploadFiles (small tree with a subfolder, progress payloads)
        let local = FileManager.default.temporaryDirectory
            .appendingPathComponent("keel-golden-src", isDirectory: true)
        await step("UploadFiles") {
            let sub = local.appendingPathComponent("sub", isDirectory: true)
            try FileManager.default.createDirectory(at: sub, withIntermediateDirectories: true)
            try Data(repeating: 0xA5, count: 1_500_000)
                .write(to: local.appendingPathComponent("blob-1.5mb.bin"))
            try "hello from keel golden capture — émoji: 🛳️\n".data(using: .utf8)!
                .write(to: local.appendingPathComponent("note-🛳️.txt"))
            try Data(repeating: 0x5A, count: 300_000)
                .write(to: sub.appendingPathComponent("nested.bin"))

            let stream = await engine.transfer(
                .upload, storageId: sid, sources: [local.path], destination: base)
            for try await _ in stream {}
        }
        // 8. Walk the uploaded tree (recursive)
        await step("Walk uploaded (recursive)") {
            _ = try await engine.list(storageId: sid, path: base, recursive: true)
        }
        // 9. RenameFile
        await step("RenameFile") {
            try await engine.rename(
                storageId: sid, path: "\(base)/keel-golden-src/blob-1.5mb.bin",
                to: "blob-renamed.bin")
        }
        // 10. DownloadFiles (progress payloads)
        await step("DownloadFiles") {
            let dest = FileManager.default.temporaryDirectory
                .appendingPathComponent("keel-golden-dst", isDirectory: true)
            try FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
            let stream = await engine.transfer(
                .download, storageId: sid, sources: ["\(base)/keel-golden-src"],
                destination: dest.path)
            for try await _ in stream {}
        }
        // 11. Error-shape fixtures: operations against a missing path
        await step("Error fixtures (expected failures)") {
            _ = try? await engine.list(storageId: sid, path: "\(base)/no-such-dir")
            try? await engine.rename(
                storageId: sid, path: "\(base)/no-such-file.bin", to: "x.bin")
            _ = try? await engine.delete(storageId: sid, paths: ["\(base)/no-such-file.bin"])
        }
        // 12. DeleteFile (cleanup) + Dispose
        await step("DeleteFile (cleanup)") {
            try await engine.delete(storageId: sid, paths: [base])
        }
        try? FileManager.default.removeItem(at: local)
        await engine.dispose()
        print("golden session complete — payloads in KEEL_DUMP_DIR")
    }
}
