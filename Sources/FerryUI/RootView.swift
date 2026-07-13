import KeelKit
import SwiftUI

/// App entry view: start screen (3a/3b) until a session is live, then the
/// Shelf browser (3c+).
public struct RootView: View {
    @State private var session = DeviceSession()

    public init() {}

    public var body: some View {
        Group {
            if session.phase == .connected {
                BrowserView(session: session)
            } else {
                StartView(session: session)
            }
        }
        .frame(minWidth: 780, minHeight: 500)
        .tint(.ink)  // monochrome: prominent buttons, progress, selection read as ink, not system blue
        .task {
            // Auto-detect for the app's lifetime — the design has no
            // Connect button (3a: "it appears here automatically").
            await session.autoConnectLoop()
        }
    }
}
