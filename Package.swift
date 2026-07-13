// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "ferry",
    platforms: [
        .macOS("26.0")
    ],
    products: [
        .library(name: "KeelKit", targets: ["KeelKit"]),
        // FerryUI is a product so Xcode generates a scheme for it —
        // previews need to host in this library's scheme, not the app's.
        .library(name: "FerryUI", targets: ["FerryUI"]),
        .executable(name: "ferry-probe", targets: ["FerryProbe"]),
        .executable(name: "FerryApp", targets: ["FerryApp"]),
    ],
    targets: [
        // Swift bindings to keel.dylib (the Rust MTP kernel): FFI bridge,
        // async engine, and Codable models.
        .target(name: "KeelKit"),
        // CLI harness: loads the dylib and lists device info + storages.
        .executableTarget(
            name: "FerryProbe",
            dependencies: ["KeelKit"]
        ),
        // Views live in a library, not the executable: Xcode previews can't
        // run in executable targets (ENABLE_DEBUG_DYLIB), but work in
        // libraries out of the box. FerryApp is just the @main shell.
        .target(
            name: "FerryUI",
            dependencies: ["KeelKit"]
        ),
        .executableTarget(
            name: "FerryApp",
            dependencies: ["FerryUI", "KeelKit"]
        ),
    ]
)
