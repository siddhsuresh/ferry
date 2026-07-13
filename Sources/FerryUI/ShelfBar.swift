import KeelKit
import SwiftUI
import UniformTypeIdentifiers

/// The Shelf (3c/3d/3h/3i/3n): a persistent bottom bar that is, in turn,
/// a drop target, a gathered-cargo strip, the transfer progress tray, and
/// the quiet done card.
struct ShelfBar: View {
    @Bindable var session: DeviceSession
    @State private var dropTargeted = false

    var body: some View {
        Group {
            if let transfer = session.transfer, !isSilentPreviewFetch {
                progressTray(transfer)
            } else if let done = session.completion {
                doneCard(done)
            } else {
                shelfStrip
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
        .animation(.smooth(duration: 0.25), value: session.transfer != nil)
        .animation(.smooth(duration: 0.25), value: session.completion != nil)
    }

    /// Preview fetches are transfers too, but the wireframe keeps them out
    /// of the tray — a small spinner shows on the grid instead.
    private var isSilentPreviewFetch: Bool {
        session.isPreparingPreview && session.queue.isEmpty
    }

    // MARK: Shelf strip (3c gathered / 3d & 3n empty)

    private var shelfStrip: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 0) {
                if session.shelf.isEmpty {
                    Text("Shelf empty — drag files here (from the phone or Finder), or right-click → Add to Shelf")
                        .font(.callout)
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
                } else {
                    HStack {
                        Text("Shelf · \(session.shelf.count) item\(session.shelf.count == 1 ? "" : "s") · \(Format.bytes(session.shelfTotalBytes))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button {
                            session.clearShelf()
                        } label: {
                            Label("Clear", systemImage: "xmark.circle.fill")
                                .font(.caption)
                        }
                        .buttonStyle(.borderless)
                        .foregroundStyle(.tertiary)
                        .help("Empty the shelf")
                    }
                    .padding(.horizontal, 4)

                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(session.shelf) { item in
                                shelfChip(item)
                            }
                        }
                        .padding(.horizontal, 2)
                        .padding(.top, 4)
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            // Fixed height (min == max): the empty state's centered text used
            // to stretch the dashed box to fill all offered space.
            .frame(maxWidth: .infinity, minHeight: 88, maxHeight: 88)
            .background {
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(
                        style: StrokeStyle(lineWidth: 1.5, dash: [6, 5])
                    )
                    .foregroundStyle(dropTargeted ? AnyShapeStyle(Color.ink) : AnyShapeStyle(.quaternary))
            }
            // One drop target for BOTH phone-file drags (grid/list) and Finder
            // URL drops — a combined Transferable, so there's no nested-
            // destination conflict.
            .dropDestination(for: ShelfImport.self) { items, _ in
                let files = items.compactMap(\.phone)
                let urls = items.compactMap(\.local)
                if !files.isEmpty { session.addToShelf(files) }
                if !urls.isEmpty { session.addToShelf(urls) }
                return !files.isEmpty || !urls.isEmpty
            } isTargeted: { dropTargeted = $0 }

            if session.isSearching {
                Button("Add all \(session.searchResults.count) to Shelf") {
                    session.addAllResultsToShelf()
                }
                .buttonStyle(.glassProminent)
                .disabled(session.searchResults.isEmpty)
            } else {
                Button("Send to Mac…") { promptSendToMac() }
                    .buttonStyle(.glassProminent)
                    .disabled(session.shelfPhoneFiles.isEmpty)
                    .help("Choose a folder on this Mac, then copy the shelf's phone items there")

                Button("Send to phone") { session.sendShelfToPhone() }
                    .buttonStyle(.glass)
                    .disabled(session.shelfLocalURLs.isEmpty)
                    .help("Copy the shelf's Finder items into the current phone folder")
            }
        }
    }

    private func shelfChip(_ item: ShelfItem) -> some View {
        HStack(spacing: 7) {
            switch item {
            case .phone(let file):
                MiniTile(file: file, side: 40, session: session)
            case .local(let url):
                LocalMiniTile(url: url, side: 40)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name)
                    .font(.caption)
                    .lineLimit(2)
                    .truncationMode(.middle)
                Text(Format.bytes(item.byteSize))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            .frame(width: 104, alignment: .leading)
        }
        .padding(6)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.inkWash))
        .overlay(alignment: .topTrailing) {
            Button {
                session.removeFromShelf(item)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 15))
                    .foregroundStyle(.secondary)
                    .background(Circle().fill(.background).padding(1))
            }
            .buttonStyle(.plain)
            .offset(x: 6, y: -6)
            .help("Remove from Shelf")
        }
        .help(item.name)
        .contextMenu {
            Button("Remove from Shelf") { session.removeFromShelf(item) }
        }
    }

    /// Send to Mac always asks where — no preset destination. Starts at the
    /// last-used folder for convenience.
    private func promptSendToMac() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Save Here"
        panel.message = "Choose where to save \(session.shelfPhoneFiles.count) item(s) from the phone"
        panel.directoryURL = session.settings.destinationURL
        guard panel.runModal() == .OK, let url = panel.url else { return }
        session.sendShelfToMac(to: url)
    }

    // MARK: Progress tray (3h)

    private func progressTray(_ transfer: DeviceSession.ActiveTransfer) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Text(transfer.title)
                    .font(.callout.weight(.semibold))
                if transfer.totalFiles > 0 {
                    Text("file \(transfer.filesSent) of \(transfer.totalFiles) · \(transfer.currentFile)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                } else if transfer.currentFile.isEmpty {
                    Text("Preparing transfer…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text(transfer.currentFile)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer()
                Text(etaLine(transfer))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                Button("Cancel") { session.cancelCurrentTransfer() }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                if !session.queue.isEmpty {
                    Button("Cancel All") { session.cancelAllTransfers() }
                        .buttonStyle(.glass)
                        .controlSize(.small)
                }
            }

            if transfer.hasProgress {
                ProgressView(value: max(0, min(1, Double(transfer.progress) / 100)))
                    .progressViewStyle(.linear)
                    .animation(.linear(duration: 0.15), value: transfer.progress)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            if !session.queue.isEmpty {
                HStack(spacing: 8) {
                    Text("Next in queue:")
                    ForEach(session.queue.prefix(3)) { job in
                        Text(job.queueLabel).lineLimit(1)
                    }
                    if session.queue.count > 3 {
                        Text("+\(session.queue.count - 3) more")
                    }
                }
                .font(.caption)
                .foregroundStyle(.tertiary)
            }
        }
    }

    private func etaLine(_ t: DeviceSession.ActiveTransfer) -> String {
        var parts: [String] = []
        if t.speedMBps > 0 { parts.append(Format.speed(t.speedMBps)) }
        if let eta = t.etaSeconds { parts.append(Format.eta(eta)) }
        return parts.joined(separator: " · ")
    }

    // MARK: Done card (3i)

    private func doneCard(_ done: DeviceSession.Completion) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
                .font(.title3)
                .foregroundStyle(.primary)
            VStack(alignment: .leading, spacing: 1) {
                Text(done.headline)
                    .font(.callout.weight(.semibold))
                Text("nothing was removed from the phone")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            if let reveal = done.revealURL {
                Button("Show in Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([reveal])
                }
                .buttonStyle(.glassProminent)
                .controlSize(.small)
            }
            Button("Dismiss") { session.dismissCompletion() }
                .buttonStyle(.glass)
                .controlSize(.small)
        }
    }
}
