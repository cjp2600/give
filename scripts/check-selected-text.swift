// Regression probe for selection readers. Run with an app PID and expected text.
import AppKit
import ApplicationServices

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    let error = AXUIElementCopyAttributeValue(element, name as CFString, &value)
    if error != .success { fputs("\(name): AX error \(error.rawValue)\n", stderr) }
    return value
}
let pid = pid_t(CommandLine.arguments[1])!
let app = AXUIElementCreateApplication(pid)
guard let focused = attribute(app, kAXFocusedUIElementAttribute) else { exit(1) }
let element = focused as! AXUIElement
let role = attribute(element, kAXRoleAttribute) as? String ?? "unknown"
guard let text = attribute(element, kAXSelectedTextAttribute) as? String else {
    fputs("FAIL: \(role) does not expose selected text\n", stderr); exit(1)
}
if CommandLine.arguments.count > 2 && !text.contains(CommandLine.arguments[2]) {
    fputs("FAIL: selected text mismatch: \(text.debugDescription)\n", stderr); exit(1)
}
print("PASS: \(role), AXSelectedText = \(text.debugDescription)")
