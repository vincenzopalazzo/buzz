//! Sonar chat receipt lines (`docs/SONAR-PAYMENTS.md` in Sonar).
//!
//! Buzz and Sonar clients render a message that is exactly `⚡PAY|1|<id>|<sats>`
//! as a payment bubble, and use `⚡PAYDONE|2|<id>[|<preimage>]` (hidden) to mark
//! it settled. This module only formats those strings.

/// `⚡PAY|1|<id>|<sats>`: the visible receipt.
pub fn pay_line(id: &str, sats: u64) -> String {
    format!("⚡PAY|1|{id}|{sats}")
}

/// `⚡PAYDONE|2|<id>` or `⚡PAYDONE|2|<id>|<preimage>`: the settlement line.
pub fn done_line(id: &str, preimage: Option<&str>) -> String {
    match preimage {
        Some(p) => format!("⚡PAYDONE|2|{id}|{p}"),
        None => format!("⚡PAYDONE|2|{id}"),
    }
}

/// A fresh receipt id: 16 lowercase hex chars, like Sonar's `randomPayId()`.
pub fn new_receipt_id() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    id[..16].to_string()
}

/// Lowercase 64-char hex, as Sonar requires for a preimage.
pub fn normalize_preimage(preimage: &str) -> Option<String> {
    let p = preimage.trim().to_ascii_lowercase();
    (p.len() == 64 && p.bytes().all(|b| b.is_ascii_hexdigit())).then_some(p)
}

/// Whole sats a payment of `msat` shows in the bubble, rounded up so a
/// sub-sat payment never renders as 0 (Sonar rejects a 0-sat `⚡PAY`).
pub fn bubble_sats(msat: u64) -> u64 {
    msat.div_ceil(1000).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRE: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    // Byte-for-byte Sonar's `PayLine.encoded()`.
    #[test]
    fn matches_sonar_encoding() {
        assert_eq!(pay_line("abc-123", 21), "⚡PAY|1|abc-123|21");
        assert_eq!(done_line("abc-123", None), "⚡PAYDONE|2|abc-123");
        assert_eq!(
            done_line("abc-123", Some(PRE)),
            format!("⚡PAYDONE|2|abc-123|{PRE}")
        );
    }

    #[test]
    fn ids_are_sixteen_hex_and_unique() {
        let id = new_receipt_id();
        assert_eq!(id.len(), 16);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(id, new_receipt_id());
    }

    #[test]
    fn preimage_and_amount_normalization() {
        assert_eq!(normalize_preimage(&PRE.to_uppercase()), Some(PRE.into()));
        assert_eq!(normalize_preimage("abc"), None);
        assert_eq!(bubble_sats(21_000), 21);
        assert_eq!(bubble_sats(21_001), 22);
        assert_eq!(bubble_sats(1), 1);
        assert_eq!(bubble_sats(0), 1);
    }
}
