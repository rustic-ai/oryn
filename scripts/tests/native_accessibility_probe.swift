// Source-only platform probe. Compile/run only after the global generation gate.
// Requires the testing process to have Accessibility permission already granted.
import AppKit
import ApplicationServices
import Foundation

struct ProbeError: Error, CustomStringConvertible { let description: String }
func fail(_ text: String) throws -> Never { throw ProbeError(description: text) }
func value(_ element: AXUIElement, _ name: CFString) throws -> CFTypeRef? {
    var result: CFTypeRef?
    let status = AXUIElementCopyAttributeValue(element, name, &result)
    if status == .noValue || status == .attributeUnsupported { return nil }
    guard status == .success else { try fail("AX read \(name): \(status.rawValue)") }
    return result
}
func children(_ element: AXUIElement) throws -> [AXUIElement] {
    return try value(element, kAXChildrenAttribute as CFString) as? [AXUIElement] ?? []
}
func walk(_ root: AXUIElement, _ depth: Int = 0, _ count: inout Int) throws -> [AXUIElement] {
    guard depth < 128, count < 65536 else { try fail("AX traversal budget exceeded") }
    count += 1
    var result = [root]
    for child in try children(root) { result += try walk(child, depth + 1, &count) }
    return result
}
func string(_ element: AXUIElement, _ name: String) throws -> String? {
    return try value(element, name as CFString) as? String
}
do {
    guard CommandLine.arguments.count >= 2, let pid = Int32(CommandLine.arguments[1]) else {
        try fail("usage: native_accessibility_probe PID [button-label-to-press]")
    }
    guard AXIsProcessTrusted() else { try fail("probe lacks existing macOS Accessibility permission") }
    let application = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(application, 5)
    var count = 0
    let nodes = try walk(application, 0, &count)
    let native = try nodes.filter { try string($0, "AXIdentifier")?.hasPrefix("oryn:") == true }
    guard !native.isEmpty else { try fail("no native Oryn accessibility elements exposed") }
    var identifiers = Set<String>()
    for node in native {
        guard let identifier = try string(node, "AXIdentifier"), identifiers.insert(identifier).inserted else {
            try fail("missing or duplicated native generation identity")
        }
        guard let role = try string(node, "AXRole"), role.hasPrefix("AX") else { try fail("missing OS role") }
        if let position = try value(node, kAXPositionAttribute as CFString) {
            guard CFGetTypeID(position) == AXValueGetTypeID() else { try fail("non-native AX position") }
        }
    }
    if CommandLine.arguments.count > 2 {
        let label = CommandLine.arguments[2]
        let matches = try native.filter { try string($0, "AXTitle") == label }
        guard matches.count == 1 else { try fail("press target is absent or ambiguous") }
        let status = AXUIElementPerformAction(matches[0], kAXPressAction as CFString)
        guard status == .success else { try fail("native AX press failed: \(status.rawValue)") }
        // Acceptance here means OS delivery only; durable page effects need a fixture oracle.
    }
    let report: [String: Any] = ["schema": 1, "pid": pid, "native_element_count": native.count,
        "os_attribute_delivery": true, "durable_action_effect_verified": false]
    let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
    print(String(decoding: data, as: UTF8.self))
} catch { fputs("\(error)\n", stderr); exit(1) }
