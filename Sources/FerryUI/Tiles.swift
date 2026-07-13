import KeelKit
import SwiftUI
import UniformTypeIdentifiers

// Drag payload type for phone files → shelf (wireframe 3c). Declared in the
// app's Info.plist (UTExportedTypeDeclarations, added by make-app.sh) so the
// drag/drop system recognises it; conforms to public.data.
extension UTType {
    static let ferryPhoneFile = UTType(exportedAs: "com.siddharth.ferry.phonefile")
}

extension KeelFile: Transferable {
    public static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: .ferryPhoneFile)
    }
}

/// One shelf drop payload — a phone file (dragged from the grid/list, arriving
/// as `.ferryPhoneFile`-typed JSON) or a Finder URL. A single Transferable lets
/// the shelf use ONE drop target for both; two separate typed destinations were
/// conflicting and neither fired.
struct ShelfImport: Transferable {
    let phone: KeelFile?
    let local: URL?

    static var transferRepresentation: some TransferRepresentation {
        // Phone file: the same `.ferryPhoneFile` JSON the grid/list drag emits.
        DataRepresentation(contentType: .ferryPhoneFile) { item in
            (try? JSONEncoder().encode(item.phone)) ?? Data()
        } importing: { data in
            ShelfImport(phone: try JSONDecoder().decode(KeelFile.self, from: data), local: nil)
        }
        // Finder file: a plain file URL.
        ProxyRepresentation(importing: { (url: URL) in ShelfImport(phone: nil, local: url) })
    }
}

extension KeelFile {
    var extLabel: String {
        let e = `extension`.trimmingCharacters(in: CharacterSet(charactersIn: "."))
        return e.isEmpty ? "FILE" : e.uppercased()
    }

    var category: FileCategory {
        for c in FileCategory.allCases where c != .all && c.matches(self) { return c }
        return .all
    }

    /// Tile glyph per the wireframe: ▶ video, ♪ audio, ext badge otherwise.
    /// Type is carried by shape, not color (monochrome).
    var tileGlyph: String? {
        switch category {
        case .video: return "play.fill"
        case .audio: return "music.note"
        case .images: return "photo"
        default: return nil
        }
    }
}

/// Square tile for the grid (3c): a real device thumbnail for images/videos
/// (fetched lazily over MTP as the tile appears), else a folder glyph or
/// type-badged card.
struct FileTile: View {
    let file: KeelFile
    var session: DeviceSession
    @State private var thumbnail: NSImage?

    private var wantsThumbnail: Bool {
        file.category == .images || file.category == .video
    }

    var body: some View {
        VStack(spacing: 4) {
            // The base rect (with aspect 1) is THE square, sized by the grid
            // cell. The thumbnail rides as an overlay so a landscape image
            // fills and is clipped to the square instead of expanding the cell.
            RoundedRectangle(cornerRadius: 10)
                .fill(Color.inkWash)
                .aspectRatio(1, contentMode: .fit)
                .overlay {
                    if let thumbnail {
                        Image(nsImage: thumbnail)
                            .resizable()
                            .interpolation(.medium)
                            .scaledToFill()
                    } else if file.isFolder {
                        Image(systemName: "folder.fill")
                            .font(.system(size: 30))
                            .foregroundStyle(.primary)
                    } else if let glyph = file.tileGlyph {
                        Image(systemName: glyph)
                            .font(.system(size: 26))
                            .foregroundStyle(.secondary)
                    } else {
                        Text(file.extLabel)
                            .font(.system(size: 15, weight: .bold, design: .rounded))
                            .foregroundStyle(.secondary)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay {
                    // Play badge marks videos once the frame loads.
                    if file.category == .video, thumbnail != nil {
                        Image(systemName: "play.circle.fill")
                            .font(.system(size: 22))
                            .foregroundStyle(.white.opacity(0.9))
                            .shadow(radius: 2)
                    }
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 10)
                        .strokeBorder(.quaternary, lineWidth: 1)
                }

            Text(file.name)
                .font(.system(size: 11.5))
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: .infinity)
        }
        // Tied to the tile's lifetime in the LazyVGrid: cancels when the tile
        // scrolls away or the folder changes, so only visible images fetch.
        .task(id: file.objectId) {
            guard wantsThumbnail, thumbnail == nil else { return }
            thumbnail = await session.thumbnailImage(for: file)
        }
    }
}

/// Small square tile for lists, search results, and the shelf strip. When a
/// `session` is supplied it lazily loads a real thumbnail for images/videos
/// (cached, same as the grid); otherwise it shows the type glyph.
struct MiniTile: View {
    let file: KeelFile
    var side: CGFloat = 28
    var session: DeviceSession?
    @State private var thumbnail: NSImage?

    private var wantsThumbnail: Bool {
        session != nil && (file.category == .images || file.category == .video)
    }

    var body: some View {
        RoundedRectangle(cornerRadius: 6)
            .fill(Color.inkWash)
            .overlay {
                if let thumbnail {
                    Image(nsImage: thumbnail)
                        .resizable()
                        .interpolation(.low)
                        .scaledToFill()
                } else if file.isFolder {
                    Image(systemName: "folder.fill")
                        .font(.system(size: side * 0.42))
                        .foregroundStyle(.primary)
                } else if let glyph = file.tileGlyph {
                    Image(systemName: glyph)
                        .font(.system(size: side * 0.38))
                        .foregroundStyle(.secondary)
                } else {
                    Text(file.extLabel)
                        .font(.system(size: side * 0.26, weight: .bold, design: .rounded))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                        .padding(.horizontal, 2)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .frame(width: side, height: side)
            .task(id: file.objectId) {
                guard wantsThumbnail, thumbnail == nil else { return }
                thumbnail = await session?.thumbnailImage(for: file)
            }
    }
}

/// Mini tile for local (Finder) shelf items.
struct LocalMiniTile: View {
    let url: URL
    var side: CGFloat = 28

    var body: some View {
        Image(nsImage: NSWorkspace.shared.icon(forFile: url.path))
            .resizable()
            .frame(width: side, height: side)
    }
}

enum Format {
    static func bytes(_ count: Int64?) -> String {
        guard let count, count > 0 else { return "—" }
        return ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    static func speed(_ mbps: Double) -> String {
        String(format: "%.0f MB/s", mbps)
    }

    /// 3h "2 min left"
    static func eta(_ seconds: Int) -> String {
        if seconds < 60 { return "\(seconds) sec left" }
        return "\(Int((Double(seconds) / 60).rounded())) min left"
    }

    /// The kernel's dateAdded is ISO-ish; show "Jul 2, 2026" like 3d.
    static func date(_ raw: String) -> String {
        let iso = ISO8601DateFormatter()
        let parsed = iso.date(from: raw)
            ?? ISO8601DateFormatter.withFractional.date(from: raw)
        guard let parsed else { return raw }
        return parsed.formatted(.dateTime.month(.abbreviated).day().year())
    }
}

extension ISO8601DateFormatter {
    nonisolated(unsafe) static let withFractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
}
