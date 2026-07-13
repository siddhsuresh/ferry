import AppKit
import SwiftUI

/// 3m — "Settings — tiny on purpose."
public struct SettingsView: View {
    @Bindable private var settings = FerrySettings.shared

    public init() {}

    public var body: some View {
        Form {
            LabeledContent("Save phone files to:") {
                Button {
                    pickDestination()
                } label: {
                    HStack(spacing: 5) {
                        Text(settings.destinationPath)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Image(systemName: "chevron.down")
                            .font(.caption2)
                    }
                }
            }

            Toggle("Play a sound when a transfer finishes", isOn: $settings.playSound)
            Toggle("Open Finder when done", isOn: $settings.revealInFinder)
            Toggle("Skip files that already exist on the Mac", isOn: $settings.skipExisting)
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .fixedSize()
    }

    private func pickDestination() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Save Here"
        panel.directoryURL = FerrySettings.shared.destinationURL
        guard panel.runModal() == .OK, let url = panel.url else { return }
        settings.destinationPath =
            url.path.replacingOccurrences(
                of: FileManager.default.homeDirectoryForCurrentUser.path,
                with: "~")
    }
}
