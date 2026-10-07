//! Sonar chat receipt lines, mirroring
//! `desktop/src/features/messages/lib/sonarPay.ts`.
//!
//! A message made only of `⚡PAYDONE|…` lines settles an earlier `⚡PAY`
//! receipt. The timeline hides it, so native unread and feed paths must
//! skip it too.

const CARRIER_KINDS: [u16; 2] = [9, 40002];
const PAYDONE: &str = "⚡PAYDONE";

/// True when a chat message carries only `⚡PAYDONE` lines, i.e. a hidden
/// settlement row that must not count as unread or appear in the feed.
pub(crate) fn is_control_message(kind: u16, content: &str) -> bool {
    if !CARRIER_KINDS.contains(&kind) {
        return false;
    }
    let mut saw_done = false;
    for line in content.split('\n') {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !is_done_line(line) {
            return false;
        }
        saw_done = true;
    }
    saw_done
}

fn is_done_line(line: &str) -> bool {
    let parts: Vec<&str> = line.split('|').collect();
    match parts.as_slice() {
        [PAYDONE, "1" | "2", id] => !id.is_empty(),
        [PAYDONE, "2", id, preimage] => !id.is_empty() && is_hex64(preimage),
        _ => false,
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::is_control_message;

    const PREIMAGE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn done_only_messages_are_control_rows() {
        assert!(is_control_message(9, "⚡PAYDONE|2|abc"));
        assert!(is_control_message(40002, "⚡PAYDONE|1|abc\n"));
        assert!(is_control_message(
            9,
            &format!("⚡PAYDONE|2|a|{PREIMAGE}  \n\n⚡PAYDONE|2|b")
        ));
    }

    #[test]
    fn receipts_text_and_malformed_lines_stay_visible() {
        assert!(!is_control_message(9, "⚡PAY|1|abc|21\n⚡PAYDONE|2|abc"));
        assert!(!is_control_message(9, "Paid.\n⚡PAYDONE|2|abc"));
        assert!(!is_control_message(9, " ⚡PAYDONE|2|abc"));
        assert!(!is_control_message(9, "⚡PAYDONE|3|abc"));
        assert!(!is_control_message(9, "⚡PAYDONE|1|abc|ff"));
        assert!(!is_control_message(9, "⚡PAYDONE|2|abc|not-hex"));
        assert!(!is_control_message(9, "⚡PAYDONE|2|"));
        assert!(!is_control_message(9, ""));
    }

    #[test]
    fn only_chat_kinds_carry_receipts() {
        assert!(!is_control_message(40003, "⚡PAYDONE|2|abc"));
        assert!(!is_control_message(1, "⚡PAYDONE|2|abc"));
    }
}
