import AppKit
// Voxi app icon: a listening face (two eyes and a smile drawn as a sound wave)
// on a plain paper tile, sized to Apple's macOS icon grid (824px tile on 1024).
// The website's favicon (website/assets/favicon.svg) is the same face.
//
//   swift docs/icon/appicon.swift <icon.png> [tray.png]
//
// The optional second path also writes the 88px menu bar template icon.

func hex(_ v: UInt32, _ a: CGFloat = 1) -> NSColor {
  NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: a)
}
let ink = hex(0x1B2A4A), margin = hex(0xD2453B)

// Draws into a fresh bitmap in SVG-style coordinates (origin top-left, y down).
func render(_ size: Int, to path: String, _ draw: (CGContext) -> Void) {
  let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
  let ctx = NSGraphicsContext.current!.cgContext
  ctx.translateBy(x: 0, y: CGFloat(size)); ctx.scaleBy(x: 1, y: -1)
  draw(ctx)
  NSGraphicsContext.current = nil
  try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

// The face in a 100-unit box. `p` maps face units to pixels.
// The smile is a downward arc with three ripples riding on it, like a sound wave.
func drawFace(eyes: NSColor, mouth: NSColor, eyeWidth: Double, mouthWidth: Double, _ p: (Double, Double) -> NSPoint, _ k: Double) {
  for x in [31.0, 61.0] {
    let a = p(x, 26), b = p(x + eyeWidth, 46)
    let r = eyeWidth / 2 * k
    eyes.setFill(); NSBezierPath(roundedRect: NSRect(x: a.x, y: a.y, width: b.x - a.x, height: b.y - a.y), xRadius: r, yRadius: r).fill()
  }
  let smile = NSBezierPath(), n = 64
  for i in 0...n {
    let t = Double(i) / Double(n), s = sin(Double.pi * t)
    let pt = p(26 + 48 * t, 62 + 8 * s + 2.6 * sin(6 * Double.pi * t) * s)
    if i == 0 { smile.move(to: pt) } else { smile.line(to: pt) }
  }
  smile.lineWidth = mouthWidth * k; smile.lineCapStyle = .round; smile.lineJoinStyle = .round
  mouth.setStroke(); smile.stroke()
}

render(1024, to: CommandLine.arguments[1]) { ctx in
  // Tile with Apple's grid: 100px inset, ~185px corner radius, soft drop shadow.
  let tile = NSRect(x: 100, y: 100, width: 824, height: 824)
  let shape = NSBezierPath(roundedRect: tile, xRadius: 185, yRadius: 185)
  ctx.saveGState()
  ctx.setShadow(offset: CGSize(width: 0, height: 12), blur: 28, color: NSColor.black.withAlphaComponent(0.28).cgColor)
  hex(0xF7F9FC).setFill(); shape.fill()
  ctx.restoreGState()
  ctx.saveGState(); shape.addClip()
  NSGradient(starting: hex(0xE6EDF8), ending: hex(0xFFFFFF))!.draw(in: tile, angle: -90)
  ctx.restoreGState()
  hex(0x1B2A4A, 0.10).setStroke(); shape.lineWidth = 2; shape.stroke()

  let k = 8.24
  drawFace(eyes: ink, mouth: margin, eyeWidth: 8, mouthWidth: 5.5, { x, y in NSPoint(x: 100 + x * k, y: 100 + y * k) }, k)
}

// Menu bar template icon: black on transparent; macOS tints it. Bolder strokes so
// it holds up at 22pt, and the face is centred vertically in the frame.
if CommandLine.arguments.count > 2 {
  render(88, to: CommandLine.arguments[2]) { _ in
    let k = 1.45
    drawFace(eyes: .black, mouth: .black, eyeWidth: 9, mouthWidth: 7.5, { x, y in NSPoint(x: 44 + (x - 50) * k, y: 44 + (y - 51) * k) }, k)
  }
}
