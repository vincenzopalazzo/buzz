import Foundation

/// Sonar chat receipt lines, mirroring
/// `desktop/src/features/messages/lib/sonarPay.ts`. `⚡PAY|1|<id>|<sats>` is a
/// payment receipt; a message of only `⚡PAYDONE|…` lines is a hidden row that
/// settles one, so it must never become a notification.
enum SonarPayPreview {
  private enum Line {
    case pay(id: String, sats: Int)
    case done(id: String)
  }

  /// One-line text for a notification: receipt lines become
  /// "⚡ Paid 2,100 sats" or "⚡ 2,100 sats payment". Returns nil for a
  /// `⚡PAYDONE`-only control row, and `content` unchanged when it has no
  /// payment lines.
  static func previewText(_ content: String) -> String? {
    var kept: [String] = []
    var pays: [(id: String, sats: Int)] = []
    var settled = Set<String>()
    for rawLine in content.components(separatedBy: "\n") {
      switch decode(trimmingTrailingWhitespace(rawLine)) {
      case .pay(let id, let sats): pays.append((id: id, sats: sats))
      case .done(let id): settled.insert(id)
      case nil: kept.append(rawLine)
      }
    }
    if pays.isEmpty && settled.isEmpty { return content }
    let text = kept.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
    if pays.isEmpty { return text.isEmpty ? nil : text }
    let summaries = pays.map { pay in
      settled.contains(pay.id)
        ? "⚡ Paid \(formatSats(pay.sats)) sats" : "⚡ \(formatSats(pay.sats)) sats payment"
    }
    return ([text] + summaries).filter { !$0.isEmpty }.joined(separator: " ")
  }

  /// True for a chat message (kind 9 or 40002) of only `⚡PAYDONE` lines.
  static func isControlMessage(kind: Int, content: String) -> Bool {
    (kind == 9 || kind == 40002) && previewText(content) == nil
  }

  private static func decode(_ line: String) -> Line? {
    let parts = line.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
    guard parts.count >= 3, !parts[2].isEmpty else { return nil }
    let id = parts[2]
    switch (parts[0], parts[1]) {
    case ("⚡PAY", "1"):
      guard parts.count >= 4, !parts[3].isEmpty,
        parts[3].allSatisfy({ $0.isASCII && $0.isNumber }),
        let sats = Int(parts[3]), sats > 0
      else { return nil }
      return .pay(id: id, sats: sats)
    case ("⚡PAYDONE", "1"):
      return parts.count == 3 ? .done(id: id) : nil
    case ("⚡PAYDONE", "2"):
      if parts.count == 3 { return .done(id: id) }
      let preimage = parts.count == 4 ? parts[3] : ""
      return preimage.count == 64 && preimage.allSatisfy({ $0.isASCII && $0.isHexDigit })
        ? .done(id: id) : nil
    default:
      return nil
    }
  }

  private static func trimmingTrailingWhitespace(_ line: String) -> String {
    var line = line
    while let last = line.last, last.isWhitespace { line.removeLast() }
    return line
  }

  private static func formatSats(_ sats: Int) -> String {
    let digits = Array(String(sats))
    var result = ""
    for (index, digit) in digits.enumerated() {
      if index > 0 && (digits.count - index) % 3 == 0 { result.append(",") }
      result.append(digit)
    }
    return result
  }
}
