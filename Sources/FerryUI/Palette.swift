import AppKit
import SwiftUI

// Monochromatic palette (design turn 3 wireframe: ink on paper, no color).
// Type identity is carried by glyph/badge shape, never hue. The single "ink"
// accent mirrors the wireframe's #2b2b2b filled pills; everything else is the
// system's semantic greys (.primary/.secondary/.tertiary/.quaternary), which
// already adapt across light and dark.
extension Color {
    /// The app's one accent — charcoal in light, off-white in dark. Applied as
    /// the root `.tint`, so prominent buttons, progress, and selection all read
    /// as ink rather than system blue.
    static let ink = Color(nsColor: NSColor(name: nil) { appearance in
        let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        return dark ? NSColor(white: 0.92, alpha: 1) : NSColor(white: 0.16, alpha: 1)
    })

    /// Neutral fill for tiles/chips — a faint ink wash, not a colored tint.
    static let inkWash = Color(nsColor: NSColor(name: nil) { appearance in
        let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        return dark ? NSColor(white: 1, alpha: 0.10) : NSColor(white: 0, alpha: 0.06)
    })

    /// Text/glyph color that sits ON `ink` — the inverse (paper). Near-white in
    /// light, near-black in dark, so a filled ink pill's label always contrasts.
    static let inkContrast = Color(nsColor: NSColor(name: nil) { appearance in
        let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        return dark ? NSColor(white: 0.10, alpha: 1) : NSColor(white: 0.98, alpha: 1)
    })
}
