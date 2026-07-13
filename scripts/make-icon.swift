#!/usr/bin/env swift
// Renders the Ferry app icon — design doc 3p "Icon A: hull + cargo".
// Charcoal squircle, white ferry hull carrying a file container, wake lines.
// Scaled 8× from the approved 128pt SVG. Assembles AppIcon.icns.
//
//   swift scripts/make-icon.swift        # writes assets/AppIcon.icns

import AppKit

let canvas: CGFloat = 1024
let S: CGFloat = 8  // 128pt design → 1024px master

func drawMaster() -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: Int(canvas), pixelsHigh: Int(canvas),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    defer { NSGraphicsContext.restoreGraphicsState() }

    // SVG y-down → AppKit y-up.
    func pt(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: x * S, y: canvas - y * S) }

    // Squircle: rect x4 y4 w120 h120 rx28, fill #26262b
    let charcoal = NSColor(calibratedRed: 0x26 / 255, green: 0x26 / 255, blue: 0x2b / 255, alpha: 1)
    charcoal.setFill()
    NSBezierPath(
        roundedRect: NSRect(x: 4 * S, y: 4 * S, width: 120 * S, height: 120 * S),
        xRadius: 28 * S, yRadius: 28 * S
    ).fill()

    NSColor.white.set()

    // Container: rect x50 y38 w28 h22 rx3, stroke white 4
    let container = NSBezierPath(
        roundedRect: NSRect(x: 50 * S, y: canvas - (38 + 22) * S, width: 28 * S, height: 22 * S),
        xRadius: 3 * S, yRadius: 3 * S)
    container.lineWidth = 4 * S
    container.stroke()

    // Container roof: M56 38 v-6 h10 l4 6
    let roof = NSBezierPath()
    roof.move(to: pt(56, 38))
    roof.line(to: pt(56, 32))
    roof.line(to: pt(66, 32))
    roof.line(to: pt(70, 38))
    roof.lineWidth = 4 * S
    roof.lineJoinStyle = .round
    roof.stroke()

    // Hull: M34 66 h60 l-8 16 H42 l-8 -16 z, fill white
    let hull = NSBezierPath()
    hull.move(to: pt(34, 66))
    hull.line(to: pt(94, 66))
    hull.line(to: pt(86, 82))
    hull.line(to: pt(42, 82))
    hull.close()
    hull.fill()

    // Wake: M26 94 c6 0 6 5 12 5 s6 -5 12 -5 …, stroke 4, round cap, opacity .55
    NSColor.white.withAlphaComponent(0.55).setStroke()
    let wake = NSBezierPath()
    wake.move(to: pt(26, 94))
    var x: CGFloat = 26
    for i in 0..<6 {
        let up = i % 2 == 0
        wake.curve(
            to: pt(x + 12, up ? 99 : 94),
            controlPoint1: pt(x + 6, up ? 94 : 99),
            controlPoint2: pt(x + 6, up ? 99 : 94))
        x += 12
    }
    wake.lineWidth = 4 * S
    wake.lineCapStyle = .round
    wake.stroke()

    return rep
}

let master = drawMaster()
let masterImage = NSImage(size: NSSize(width: canvas, height: canvas))
masterImage.addRepresentation(master)

// Assemble the iconset.
let fm = FileManager.default
let root = URL(fileURLWithPath: fm.currentDirectoryPath)
let iconset = root.appendingPathComponent("assets/AppIcon.iconset")
try? fm.removeItem(at: iconset)
try! fm.createDirectory(at: iconset, withIntermediateDirectories: true)

for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let px = size * scale
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        NSGraphicsContext.current?.imageInterpolation = .high
        masterImage.draw(
            in: NSRect(x: 0, y: 0, width: px, height: px),
            from: .zero, operation: .copy, fraction: 1.0)
        NSGraphicsContext.restoreGraphicsState()

        let suffix = scale == 2 ? "@2x" : ""
        let name = "icon_\(size)x\(size)\(suffix).png"
        try! rep.representation(using: .png, properties: [:])!
            .write(to: iconset.appendingPathComponent(name))
    }
}

// iconutil → .icns
let task = Process()
task.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
task.arguments = [
    "-c", "icns", iconset.path,
    "-o", root.appendingPathComponent("assets/AppIcon.icns").path,
]
try! task.run()
task.waitUntilExit()
guard task.terminationStatus == 0 else {
    fatalError("iconutil failed")
}
print("wrote assets/AppIcon.icns")
