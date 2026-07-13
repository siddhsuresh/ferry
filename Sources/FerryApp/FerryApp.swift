import AppKit
import FerryUI
import SwiftUI

@main
struct FerryApp: App {
    init() {
        // SPM executables launch as background processes; promote to a
        // regular app so the window and menu bar appear during development.
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate()
    }

    var body: some Scene {
        WindowGroup("Ferry") {
            RootView()
        }
        .windowToolbarStyle(.unified)
        .defaultSize(width: 1000, height: 640)

        // 3m — standard macOS Settings window (⌘,)
        Settings {
            SettingsView()
        }
    }
}
