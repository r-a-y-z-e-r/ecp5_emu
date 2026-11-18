// Adapted from scratch winpid.swift; metadata only, never captures pixels.
import CoreGraphics
import Foundation
let pid = Int(CommandLine.arguments[1])!
let list = (CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]]) ?? []
func bounds(_ w: [String: Any]) -> CGRect {
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    return CGRect(x: b["X"] as? Double ?? 0, y: b["Y"] as? Double ?? 0,
                  width: b["Width"] as? Double ?? 0, height: b["Height"] as? Double ?? 0)
}
var above: [[String: Any]] = []
var result: [String: Any] = ["asleep": CGDisplayIsAsleep(CGMainDisplayID()) != 0]
for w in list {
    let r = bounds(w)
    if w[kCGWindowOwnerPID as String] as? Int == pid && w[kCGWindowLayer as String] as? Int == 0 && r.width >= 200 {
        let covering = above.filter { a in
            let owner = a[kCGWindowOwnerName as String] as? String ?? "?"
            let intersection = bounds(a).intersection(r)
            return (a[kCGWindowLayer as String] as? Int ?? -1) >= 0
                && (a[kCGWindowAlpha as String] as? Double ?? 1) > 0
                && !["Window Server", "Dock", "Control Centre", "SystemUIServer", "screencapture"].contains(owner)
                && !intersection.isNull && intersection.width * intersection.height > 0
        }
        result["rect"] = [Int(r.minX), Int(r.minY), Int(r.width), Int(r.height)]
        result["covered"] = covering.map { $0[kCGWindowOwnerName as String] as? String ?? "?" }
        result["onscreen"] = CGDisplayBounds(CGMainDisplayID()).contains(r)
        break
    }
    above.append(w)
}
let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
print(String(data: data, encoding: .utf8)!)
