import KeelKit
import SwiftUI

/// 3a (waiting, steps teach) + 3b (handshake) — no Connect button; the
/// auto-detect loop does the work.
struct StartView: View {
    @Bindable var session: DeviceSession
    @State private var showTroubleshooting = false

    var body: some View {
        ZStack {
            backdrop

            switch session.phase {
            case .handshake(let name):
                handshakeCard(deviceName: name)
            default:
                waitingCard
            }
        }
        .alert(
            "Phone unplugged",
            isPresented: .init(
                get: { session.interruption != nil },
                set: { if !$0 { session.interruption = nil } })
        ) {
            Button("OK") { session.interruption = nil }
        } message: {
            if let i = session.interruption {
                Text("\(i.made) of \(i.of) files made it. Plug back in — your shelf is kept, so one click resends the rest.")
            }
        }
    }

    // MARK: 3a — waiting

    private var waitingCard: some View {
        GlassEffectContainer(spacing: 20) {
            VStack(spacing: 18) {
                // Header strip: brand + waiting pill
                HStack {
                    BrandMark()
                    Spacer()
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.mini)
                        Text("waiting for a phone…")
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .glassEffect(.regular, in: .capsule)
                }

                // Phone — — — — Mac illustration
                HStack(spacing: 16) {
                    Image(systemName: "iphone.gen3")
                        .font(.system(size: 44, weight: .thin))
                    Text("— — — —")
                        .font(.callout)
                        .foregroundStyle(.tertiary)
                        .kerning(3)
                    Image(systemName: "macbook")
                        .font(.system(size: 44, weight: .thin))
                }
                .foregroundStyle(.secondary)
                .padding(.top, 6)

                Text("Plug your phone into this Mac")
                    .font(.title3.weight(.semibold))

                // Steps 1-2-3
                HStack(spacing: 10) {
                    stepCard(number: 1, line1: "connect the", line2: "USB cable")
                    stepCard(number: 2, line1: "tap the phone's", line2: "USB notification")
                    stepCard(number: 3, line1: "choose", line2: "File transfer", boldLine2: true)
                }

                VStack(spacing: 6) {
                    Text("it appears here automatically — no clicking")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                    Button("My phone isn't showing up") {
                        showTroubleshooting = true
                    }
                    .buttonStyle(.link)
                    .font(.caption)
                    .popover(isPresented: $showTroubleshooting, arrowEdge: .bottom) {
                        troubleshooting
                    }
                }

                if let error = session.lastError {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.leading)
                        .frame(maxWidth: 400)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                        .glassEffect(.regular, in: .rect(cornerRadius: 14))
                        .transition(.blurReplace)
                }
            }
            .padding(30)
            .frame(maxWidth: 560)
            .glassEffect(.regular, in: .rect(cornerRadius: 28))
        }
        .animation(.smooth(duration: 0.3), value: session.lastError)
    }

    private func stepCard(number: Int, line1: String, line2: String, boldLine2: Bool = false) -> some View {
        VStack(spacing: 3) {
            Text("\(number)")
                .font(.headline)
            Text(line1)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(line2)
                .font(boldLine2 ? .caption.weight(.bold) : .caption)
                .foregroundStyle(boldLine2 ? .primary : .secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 12)
        .padding(.horizontal, 8)
        .glassEffect(.regular, in: .rect(cornerRadius: 12))
    }

    private var troubleshooting: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("My phone isn't showing up").font(.headline)
            Label("Unlock the phone's screen", systemImage: "lock.open")
            Label("Pull down the notification shade and set USB to “File transfer / Android Auto”", systemImage: "bell.badge")
            Label("Quit Android File Transfer or Smart Switch if installed", systemImage: "xmark.app")
            Label("Try another cable or port — charge-only cables are common", systemImage: "cable.connector")
        }
        .font(.callout)
        .padding(16)
        .frame(width: 380)
    }

    // MARK: 3b — handshake

    private func handshakeCard(deviceName: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: "iphone.gen3")
                .font(.system(size: 44))
                .foregroundStyle(.tint)
                .symbolEffect(.breathe, options: .repeating)
            Text("\(deviceName) found")
                .font(.title3.weight(.semibold))
            ProgressView()
                .progressViewStyle(.linear)
                .frame(width: 220)
            Text("Reading storage… if the phone asks “Allow access?”, tap Allow")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(36)
        .glassEffect(.regular, in: .rect(cornerRadius: 24))
        .transition(.blurReplace)
    }

    private var backdrop: some View {
        MeshGradient(
            width: 3, height: 3,
            points: [
                [0, 0], [0.5, 0], [1, 0],
                [0, 0.5], [0.55, 0.45], [1, 0.5],
                [0, 1], [0.5, 1], [1, 1],
            ],
            colors: [
                .ink.opacity(0.10), .ink.opacity(0.05), .ink.opacity(0.09),
                .ink.opacity(0.04), .clear, .ink.opacity(0.06),
                .ink.opacity(0.08), .ink.opacity(0.04), .ink.opacity(0.10),
            ]
        )
        .ignoresSafeArea()
        .background(.background)
    }
}

/// The Ferry app icon used as the brand mark in headers (3a/3c).
struct BrandMark: View {
    var size: CGFloat = 22

    var body: some View {
        Image(nsImage: NSApplication.shared.applicationIconImage)
            .resizable()
            .interpolation(.high)
            .frame(width: size, height: size)
            .accessibilityLabel("Ferry")
    }
}
