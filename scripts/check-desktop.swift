// macOS integration smoke test. Uses only disposable repositories supplied as
// arguments; starts and terminates its own app and restores the clipboard.
import AppKit
import ApplicationServices
import CoreGraphics

let executable = URL(fileURLWithPath: CommandLine.arguments[1])
let repo = CommandLine.arguments[2]
let app = Process()
app.executableURL = executable
app.arguments = [repo]
let log = FileHandle(forWritingAtPath: "/dev/null")!
app.standardOutput = log
app.standardError = log
let pasteboard = NSPasteboard.general
let previous = (pasteboard.pasteboardItems ?? []).map { item in
    item.types.compactMap { type in item.data(forType:type).map { (type, $0) } }
}
func cleanup() {
    if app.isRunning { app.terminate() }
    pasteboard.clearContents()
    let items = previous.map { entries -> NSPasteboardItem in
        let item = NSPasteboardItem(); for (type, data) in entries { item.setData(data,forType:type) }; return item
    }
    pasteboard.writeObjects(items)
}
defer { cleanup() }
try app.run()
let pid = app.processIdentifier
let axApp = AXUIElementCreateApplication(pid)
func attr(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?; AXUIElementCopyAttributeValue(element, name as CFString, &value); return value
}
func wait(_ predicate: () -> Bool) -> Bool {
    let until = Date().addingTimeInterval(8)
    while Date() < until { if predicate() { return true }; Thread.sleep(forTimeInterval:0.05) }
    return false
}
func check(_ predicate: Bool, _ label: String) {
    if !predicate {
        if let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly,0) as? [[String:Any]], let window = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == pid }), let id = window[kCGWindowNumber as String] as? Int {
            let capture = Process(); capture.executableURL = URL(fileURLWithPath:"/usr/sbin/screencapture"); capture.arguments = ["-x","-l",String(id),"/tmp/give-desktop-failure.png"]; try? capture.run(); capture.waitUntilExit()
        }
        fputs("FAIL: \(label)\n", stderr); cleanup(); exit(1)
    }; print("PASS: \(label)")
}
check(wait { attr(axApp,"AXWindows") != nil }, "app window started")
NSRunningApplication(processIdentifier:pid)?.activate(options:[.activateAllWindows])
Thread.sleep(forTimeInterval:0.5)
func key(_ code: CGKeyCode, _ flags: CGEventFlags = [], global: Bool = false) {
    for down in [true,false] {
        let event = CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down)!
        event.flags = flags; if global { event.post(tap:.cghidEventTap) } else { event.postToPid(pid) }
    }
    Thread.sleep(forTimeInterval:0.15)
}
func paste(_ text: String, global: Bool = false) {
    pasteboard.clearContents(); pasteboard.setString(text,forType:.string); key(9,.maskCommand,global:global)
}
func selected() -> String? {
    guard let focused = attr(axApp,"AXFocusedUIElement") else { return nil }
    return attr(focused as! AXUIElement,"AXSelectedText") as? String
}
key(48); key(0,.maskCommand)
check(wait { selected()?.contains("Selection reader test") == true }, "preview AXSelectedText including Cyrillic")
key(8,.maskCommand)
check(pasteboard.string(forType:.string)?.contains("Selection reader test") == true, "preview Cmd+C")
key(11,.maskCommand)
Thread.sleep(forTimeInterval:1.5)
paste("printf 'Terminal reader test\\n'"); key(36)
Thread.sleep(forTimeInterval:0.8)
key(0,.maskCommand)
check(wait { selected()?.contains("Terminal reader test") == true }, "terminal AXSelectedText")
key(8,.maskCommand)
check(pasteboard.string(forType:.string)?.contains("Terminal reader test") == true, "terminal Cmd+C")
check(selected()?.contains("Terminal reader test") == true, "terminal retains selection after copying")
// Validate that image paste is a quoted file path, then read it through the shell.
key(8,.maskControl)
paste("printf '%s' ")
let image = NSImage(size:NSSize(width:4,height:4))
image.lockFocus(); NSColor.blue.setFill(); NSRect(x:0,y:0,width:4,height:4).fill(); image.unlockFocus()
let png = NSBitmapImageRep(data:image.tiffRepresentation!)!.representation(using:.png,properties:[:])!
pasteboard.clearContents(); pasteboard.setData(png,forType:.png)
key(9,.maskCommand)
Thread.sleep(forTimeInterval:0.4)
// Typing after asynchronous image paste must stay after its file path.
// Append the suffix by individual physical key events only through clipboard.
paste(" > /tmp/give-pasted-image-path.txt")
key(36)
let saved = URL(fileURLWithPath:"/tmp/give-pasted-image-path.txt")
check(wait { (try? String(contentsOf:saved,encoding:.utf8))?.contains("give-clipboard-") == true }, "image path inserted into shell")
let path = try String(contentsOf:saved,encoding:.utf8)
check(try Data(contentsOf:URL(fileURLWithPath:path)) == png, "clipboard image persisted without changing bytes")
try? FileManager.default.removeItem(at:saved)
try? FileManager.default.removeItem(at:URL(fileURLWithPath:path).deletingLastPathComponent())
print("Desktop selection and clipboard checks passed")

