import AppKit
import KeelKit
import SwiftUI
import QuickLook
import UniformTypeIdentifiers

/// 3c–3g: header, filter chips, grid/list browsing, search, preview — with
/// the Shelf riding along the bottom.
struct BrowserView: View {
    @Bindable var session: DeviceSession

    @State private var selection = Set<UInt32>()
    @State private var sortOrder = [KeyPathComparator(\KeelFile.name)]
    @State private var newFolderName = ""
    @State private var showNewFolder = false
    @State private var renameTarget: KeelFile?
    @State private var renameValue = ""
    @State private var deleteCandidates: [KeelFile] = []
    @State private var isDropTargeted = false
    @FocusState private var searchFocused: Bool

    private var selectedFiles: [KeelFile] {
        session.files.filter { selection.contains($0.objectId) }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            chipsRow
            content
            ShelfBar(session: session)
        }
        .quickLookPreview($session.previewURL)
        .alert("New Folder", isPresented: $showNewFolder) {
            TextField("Name", text: $newFolderName)
            Button("Create") {
                Task { await session.makeFolder(named: newFolderName) }
                newFolderName = ""
            }
            Button("Cancel", role: .cancel) { newFolderName = "" }
        }
        .alert(
            "Rename",
            isPresented: .init(
                get: { renameTarget != nil },
                set: { if !$0 { renameTarget = nil } })
        ) {
            TextField("New name", text: $renameValue)
            Button("Save") {
                if let target = renameTarget {
                    Task { await session.rename(target, to: renameValue) }
                }
                renameTarget = nil
            }
            Button("Cancel", role: .cancel) { renameTarget = nil }
        }
        .confirmationDialog(
            deleteCandidates.count == 1
                ? "Delete “\(deleteCandidates.first?.name ?? "")” from the phone?"
                : "Delete \(deleteCandidates.count) items from the phone?",
            isPresented: .init(
                get: { !deleteCandidates.isEmpty },
                set: { if !$0 { deleteCandidates = [] } })
        ) {
            Button("Delete", role: .destructive) {
                let doomed = deleteCandidates
                deleteCandidates = []
                selection.subtract(doomed.map(\.objectId))
                Task { await session.delete(doomed) }
            }
            Button("Cancel", role: .cancel) { deleteCandidates = [] }
        } message: {
            Text("This can't be undone.")
        }
    }

    // MARK: - Header (3c / 3f)

    @ViewBuilder
    private var header: some View {
        HStack(spacing: 10) {
            if session.isSearching {
                searchHeader
            } else {
                BrandMark()
                devicePill
                breadcrumb
                Spacer(minLength: 12)
                searchButton
                overflowMenu
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .background(.bar)
    }

    private var searchHeader: some View {
        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Search everything on the phone…", text: $session.searchQuery)
                .textFieldStyle(.plain)
                .focused($searchFocused)
                .onSubmit { /* live filtering — nothing to submit */ }
            if session.isIndexing {
                ProgressView().controlSize(.small)
                Text("indexing…")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            Button {
                session.exitSearch()
            } label: {
                Image(systemName: "xmark")
            }
            .buttonStyle(.glass)
            .keyboardShortcut(.escape, modifiers: [])
        }
        .onAppear { searchFocused = true }
    }

    private var devicePill: some View {
        Menu {
            storagePopoverContent
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "iphone.gen3")
                Text(session.device?.displayName ?? "Phone")
                    .lineLimit(1)
            }
            .font(.callout)
        }
        .menuStyle(.button)
        .buttonStyle(.glass)
        .fixedSize()
        .help("Switch storage · see what's eating space")
    }

    /// 3l — storage switcher + capacity bar. Per-category breakdown needs a
    /// full index (gap list); shows used/free today.
    @ViewBuilder
    private var storagePopoverContent: some View {
        ForEach(session.storages) { storage in
            Button {
                session.selectedStorageID = storage.sid
            } label: {
                HStack {
                    if storage.sid == session.selectedStorageID {
                        Image(systemName: "checkmark")
                    }
                    Text(storage.description)
                    Spacer()
                    if let free = storage.freeSpace {
                        Text("\(Format.bytes(free)) free")
                    }
                }
            }
        }
        Divider()
        if let s = session.selectedStorage, let max = s.maxCapacity, let free = s.freeSpace {
            Text("\(Format.bytes(max - free)) used of \(Format.bytes(max))")
        }
        Button("Disconnect") {
            Task { await session.disconnect() }
        }
    }

    private var breadcrumb: some View {
        HStack(spacing: 3) {
            Button {
                Task { await session.navigateToRoot() }
            } label: {
                Text(session.selectedStorage?.description ?? "Internal")
                    .lineLimit(1)
            }
            .buttonStyle(.borderless)
            ForEach(Array(session.pathComponents.enumerated()), id: \.offset) { index, part in
                Text("›").foregroundStyle(.tertiary)
                Button(part) {
                    let target = "/" + session.pathComponents.prefix(index + 1).joined(separator: "/")
                    Task { await session.navigate(to: target) }
                }
                .buttonStyle(.borderless)
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .lineLimit(1)
    }

    private var searchButton: some View {
        Button {
            session.enterSearch()
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                Text("Search everything…")
                    .foregroundStyle(.tertiary)
            }
            .font(.callout)
        }
        .buttonStyle(.glass)
        .keyboardShortcut("f")
        .help("Search the whole phone (⌘F)")
    }

    private var overflowMenu: some View {
        Menu {
            Button("New folder…") { showNewFolder = true }
            Button("Refresh") { Task { await session.refresh() } }
                .keyboardShortcut("r")
            Divider()
            SettingsLink { Text("Settings…") }
            Divider()
            Button("Disconnect") { Task { await session.disconnect() } }
        } label: {
            Image(systemName: "ellipsis")
        }
        .menuStyle(.button)
        .buttonStyle(.glass)
        .fixedSize()
    }

    // MARK: - Chips row (3c / 3f)

    private var chipsRow: some View {
        HStack(spacing: 6) {
            ForEach(FileCategory.allCases) { cat in
                chip(cat)
            }
            Spacer()
            if !session.isSearching {
                Picker("View", selection: $session.viewMode) {
                    Image(systemName: "square.grid.2x2").tag(DeviceSession.ViewMode.grid)
                    Image(systemName: "list.bullet").tag(DeviceSession.ViewMode.list)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()

                Button {
                    Task { await session.navigateUp() }
                } label: {
                    Image(systemName: "chevron.up")
                }
                .buttonStyle(.borderless)
                .disabled(session.path == "/")
                .keyboardShortcut(.upArrow, modifiers: .command)
                .help("Enclosing folder (⌘↑)")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .overlay(alignment: .bottom) { Divider() }
    }

    private func chip(_ cat: FileCategory) -> some View {
        let active = session.isSearching
            ? session.searchCategory == cat : session.category == cat
        let count = session.isSearching && !session.searchQuery.isEmpty
            ? session.searchCount(for: cat) : nil

        return Button {
            if session.isSearching {
                session.searchCategory = cat
            } else {
                session.category = cat
            }
        } label: {
            HStack(spacing: 4) {
                Text(cat.rawValue)
                if let count {
                    Text("(\(count))")
                }
            }
            .font(.caption.weight(active ? .semibold : .regular))
            .padding(.horizontal, 11)
            .padding(.vertical, 4)
            .background(
                active ? AnyShapeStyle(Color.ink) : AnyShapeStyle(.quaternary.opacity(0.5)),
                in: .capsule)
            .foregroundStyle(active ? Color.inkContrast : .primary)
        }
        .buttonStyle(.plain)
    }

    // MARK: - Content

    @ViewBuilder
    private var content: some View {
        if session.isSearching {
            searchResults
        } else if session.viewMode == .grid {
            gridView
        } else {
            listView
        }
    }

    // MARK: Grid (3c)

    private var gridView: some View {
        ScrollView {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 104, maximum: 140), spacing: 12)],
                spacing: 14
            ) {
                ForEach(session.visibleFiles) { file in
                    FileTile(file: file, session: session)
                        .padding(4)
                        .background(
                            selection.contains(file.objectId)
                                ? AnyShapeStyle(Color.ink.opacity(0.12))
                                : AnyShapeStyle(.clear),
                            in: .rect(cornerRadius: 12)
                        )
                        .overlay {
                            if selection.contains(file.objectId) {
                                RoundedRectangle(cornerRadius: 12)
                                    .strokeBorder(Color.ink, lineWidth: 2)
                            }
                        }
                        .draggable(file)
                        .onTapGesture(count: 2) { openOrPreview(file) }
                        .onTapGesture { toggleSelect(file) }
                        .contextMenu { fileContextMenu(for: contextFiles(file)) }
                }
            }
            .padding(14)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(Rectangle())
        .onTapGesture { selection.removeAll() }
        .onKeyPress(.space) {
            if let file = selectedFiles.first { session.preview(file) }
            return .handled
        }
        .onDeleteCommand {
            if !selectedFiles.isEmpty { deleteCandidates = selectedFiles }
        }
        .dropDestination(for: URL.self) { urls, _ in
            session.uploadHere(urls)
            return true
        } isTargeted: { isDropTargeted = $0 }
        .overlay { contentOverlay }
    }

    private func toggleSelect(_ file: KeelFile) {
        if NSEvent.modifierFlags.contains(.command) {
            if selection.contains(file.objectId) {
                selection.remove(file.objectId)
            } else {
                selection.insert(file.objectId)
            }
        } else {
            selection = [file.objectId]
        }
    }

    private func openOrPreview(_ file: KeelFile) {
        if file.isFolder {
            Task { await session.open(file) }
        } else {
            session.preview(file)  // 3g
        }
    }

    /// 3e "Send to Mac now…" — pick a folder each time, then copy immediately.
    private func promptSendToMacNow(_ files: [KeelFile]) {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.prompt = "Save Here"
        panel.message = "Choose where to save \(files.count) item(s) from the phone"
        panel.directoryURL = session.settings.destinationURL
        guard panel.runModal() == .OK, let url = panel.url else { return }
        session.sendToMacNow(files, to: url)
    }

    private func contextFiles(_ file: KeelFile) -> [KeelFile] {
        selection.contains(file.objectId) && selectedFiles.count > 1
            ? selectedFiles : [file]
    }

    // MARK: List (3d)

    private var listView: some View {
        Table(sortedListFiles, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("Name", value: \.name) { file in
                HStack(spacing: 8) {
                    MiniTile(file: file, side: 28, session: session)
                    Text(file.name)
                }
                .contentShape(Rectangle())
                .draggable(file)
            }
            .width(min: 220)

            TableColumn("Size", value: \.size) { file in
                Text(file.isFolder ? "—" : Format.bytes(file.size))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            .width(80)

            TableColumn("Added", value: \.dateAdded) { file in
                Text(Format.date(file.dateAdded))
                    .foregroundStyle(.secondary)
            }
            .width(110)
        }
        .contextMenu(forSelectionType: UInt32.self) { ids in
            fileContextMenu(for: session.files.filter { ids.contains($0.objectId) })
        } primaryAction: { ids in
            if let id = ids.first,
                let file = session.files.first(where: { $0.objectId == id })
            {
                openOrPreview(file)
            }
        }
        .onKeyPress(.space) {
            if let file = selectedFiles.first { session.preview(file) }
            return .handled
        }
        .onDeleteCommand {
            if !selectedFiles.isEmpty { deleteCandidates = selectedFiles }
        }
        .dropDestination(for: URL.self) { urls, _ in
            session.uploadHere(urls)
            return true
        } isTargeted: { isDropTargeted = $0 }
        .overlay { contentOverlay }
    }

    private var sortedListFiles: [KeelFile] {
        let sorted = session.visibleFiles.sorted(using: sortOrder)
        return sorted.filter(\.isFolder) + sorted.filter { !$0.isFolder }
    }

    // MARK: Search results (3f)

    private var searchResults: some View {
        List(session.searchResults) { file in
            HStack(spacing: 8) {
                MiniTile(file: file, side: 30, session: session)
                Text(file.name)
                    .lineLimit(1)
                Spacer()
                Text(file.parentPath.split(separator: "/").joined(separator: " › "))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            .contentShape(Rectangle())
            .onTapGesture(count: 2) { openOrPreview(file) }
            .draggable(file)
            .contextMenu { fileContextMenu(for: [file]) }
        }
        .listStyle(.inset)
        .overlay {
            if session.isIndexing && session.searchIndex.isEmpty {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Reading the whole phone once — search is instant after this")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else if !session.searchQuery.isEmpty && session.searchResults.isEmpty {
                ContentUnavailableView.search(text: session.searchQuery)
            }
        }
    }

    // MARK: Context menu (3e — exact order)

    @ViewBuilder
    private func fileContextMenu(for files: [KeelFile]) -> some View {
        if files.count == 1, let file = files.first {
            if file.isFolder {
                Button("Open") { Task { await session.open(file) } }
            } else {
                Button("Preview") { session.preview(file) }
            }
        }
        Button("Add to Shelf") {
            session.addToShelf(files.filter { !$0.isFolder } + files.filter(\.isFolder))
        }
        Button("Send to Mac now…") { promptSendToMacNow(files) }
        Divider()
        if files.count == 1, let file = files.first {
            Button("Rename…") {
                renameValue = file.name
                renameTarget = file
            }
        }
        Button("New folder…") { showNewFolder = true }
        Divider()
        Button("Delete from phone…", role: .destructive) {
            deleteCandidates = files
        }
    }

    // MARK: Shared overlays (3n)

    @ViewBuilder
    private var contentOverlay: some View {
        if isDropTargeted {
            RoundedRectangle(cornerRadius: 14)
                .strokeBorder(Color.ink, style: .init(lineWidth: 2, dash: [8, 6]))
                .background(Color.ink.opacity(0.06), in: .rect(cornerRadius: 14))
                .overlay {
                    Label("Drop to copy to \(session.path)", systemImage: "arrow.down.circle.fill")
                        .font(.title3.weight(.medium))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 9)
                        .glassEffect(.regular, in: .capsule)
                }
                .padding(8)
                .allowsHitTesting(false)
        } else if session.isListing || session.isPreparingPreview {
            ProgressView()
                .controlSize(.large)
                .padding(22)
                .glassEffect(.regular, in: .rect(cornerRadius: 18))
        } else if session.visibleFiles.isEmpty && !session.isSearching {
            VStack(spacing: 4) {
                Text("This folder is empty.")
                    .foregroundStyle(.secondary)
                Text("Drop files from Finder to copy them here.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
    }
}
