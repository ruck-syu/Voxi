import AppKit
// Voxi app icon: the website's folder-and-spark mark (website/assets/favicon.svg)
// on an index-card tile, sized to Apple's macOS icon grid (824px tile on 1024).
let S = 1024.0
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(S), pixelsHigh: Int(S), bitsPerSample: 8,
  samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext
// Work in SVG-style coordinates: origin top-left, y down.
ctx.translateBy(x: 0, y: S); ctx.scaleBy(x: 1, y: -1)

func hex(_ v: UInt32, _ a: CGFloat = 1) -> NSColor {
  NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: a)
}
let ink = hex(0x1B2A4A), margin = hex(0xD2453B)

// Tile with Apple's grid: 100px inset, ~185px corner radius, soft drop shadow.
let tile = NSRect(x: 100, y: 100, width: 824, height: 824)
let shape = NSBezierPath(roundedRect: tile, xRadius: 185, yRadius: 185)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: 12), blur: 28, color: NSColor.black.withAlphaComponent(0.28).cgColor)
hex(0xF7F9FC).setFill(); shape.fill()
ctx.restoreGState()
ctx.saveGState(); shape.addClip()
NSGradient(starting: hex(0xE6EDF8), ending: hex(0xFFFFFF))!.draw(in: tile, angle: -90)
// Faint index-card rules behind the mark.
hex(0xA9C1E8, 0.35).setFill()
var y = tile.minY + 132.0
while y < tile.maxY { NSRect(x: tile.minX, y: y, width: tile.width, height: 3).fill(); y += 92 }
ctx.restoreGState()
hex(0x1B2A4A, 0.10).setStroke(); shape.lineWidth = 2; shape.stroke()

// The mark: favicon.svg's 44-unit viewBox, scaled into the tile.
let k = 13.0, ox = 512 - 22 * k - 6, oy = 512 - 22 * k + 14
func p(_ x: Double, _ y: Double) -> NSPoint { NSPoint(x: ox + x * k, y: oy + y * k) }
let folder = NSBezierPath()
folder.move(to: p(6, 34)); folder.line(to: p(6, 12)); folder.line(to: p(16, 12)); folder.line(to: p(20, 16)); folder.line(to: p(30, 16))
folder.move(to: p(6, 34)); folder.line(to: p(28, 34))
folder.lineWidth = 3.2 * k; folder.lineJoinStyle = .round; folder.lineCapStyle = .round
ink.setStroke(); folder.stroke()
let spark = NSBezierPath()
spark.move(to: p(34, 15)); spark.line(to: p(36.2, 21.8)); spark.line(to: p(43, 24)); spark.line(to: p(36.2, 26.2))
spark.line(to: p(34, 33)); spark.line(to: p(31.8, 26.2)); spark.line(to: p(25, 24)); spark.line(to: p(31.8, 21.8)); spark.close()
margin.setFill(); spark.fill()

NSGraphicsContext.current = nil
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
