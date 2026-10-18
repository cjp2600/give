#!/usr/bin/env swift
// Rasterize the icon's flat SVG polygons at each export size. No image dependencies.
import AppKit
import Foundation

let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? "assets/icons")
let sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]

enum IconError: Error { case invalidSVG(String), bitmapUnavailable }

func number(_ element: XMLElement, _ name: String) -> CGFloat {
    CGFloat(Double(element.attribute(forName: name)?.stringValue ?? "0") ?? 0)
}

func color(_ value: String) throws -> CGColor {
    guard value.hasPrefix("#"), value.count == 7,
          let hex = UInt32(value.dropFirst(), radix: 16) else {
        throw IconError.invalidSVG("Expected a six-digit fill color: \(value)")
    }
    return CGColor(colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!, components: [
        CGFloat((hex >> 16) & 255) / 255,
        CGFloat((hex >> 8) & 255) / 255,
        CGFloat(hex & 255) / 255, 1
    ])!
}

func render(_ source: URL, to target: URL, pixels: Int) throws {
    let document = try XMLDocument(contentsOf: source)
    guard let svg = document.rootElement(), svg.attribute(forName: "viewBox")?.stringValue == "0 0 1024 1024" else {
        throw IconError.invalidSVG(source.path)
    }
    guard let context = CGContext(data: nil, width: pixels, height: pixels,
        bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
        throw IconError.bitmapUnavailable
    }
    context.translateBy(x: 0, y: CGFloat(pixels))
    context.scaleBy(x: CGFloat(pixels) / 1024, y: -CGFloat(pixels) / 1024)
    context.setAllowsAntialiasing(true)
    for case let element as XMLElement in svg.children ?? [] {
        guard let fill = element.attribute(forName: "fill")?.stringValue else { continue }
        context.setFillColor(try color(fill))
        switch element.name {
        case "polygon":
            let values = (element.attribute(forName: "points")?.stringValue ?? "")
                .split(whereSeparator: { $0 == "," || $0.isWhitespace }).compactMap { Double($0) }
            guard values.count >= 6, values.count % 2 == 0 else { throw IconError.invalidSVG("Invalid polygon") }
            let path = CGMutablePath()
            path.move(to: CGPoint(x: values[0], y: values[1]))
            for index in stride(from: 2, to: values.count, by: 2) {
                path.addLine(to: CGPoint(x: values[index], y: values[index + 1]))
            }
            path.closeSubpath()
            context.addPath(path)
            context.fillPath()
        case "rect":
            let rect = CGRect(x: number(element, "x"), y: number(element, "y"),
                width: number(element, "width"), height: number(element, "height"))
            let radius = number(element, "rx")
            context.addPath(CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
            context.fillPath()
        default:
            throw IconError.invalidSVG("Unsupported icon primitive: \(element.name ?? "unknown")")
        }
    }
    guard let image = context.makeImage(),
          let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        throw IconError.bitmapUnavailable
    }
    try FileManager.default.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
    try png.write(to: target)
}

try render(root.appendingPathComponent("give-master.svg"), to: root.appendingPathComponent("give-master.png"), pixels: 1024)
for variant in ["default", "light", "dark"] {
    let source = root.appendingPathComponent(variant == "light" ? "give-light.svg" : "give-dark.svg")
    for size in sizes {
        try render(source, to: root.appendingPathComponent("png/\(variant)/\(size).png"), pixels: size)
    }
    if variant != "default" {
        try render(source, to: root.appendingPathComponent("give-\(variant).png"), pixels: 1024)
    }
}
print("Rendered blue hexagon from SVG at 16–1024 px")
