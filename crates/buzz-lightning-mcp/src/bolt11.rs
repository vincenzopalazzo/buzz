//! Read the parts of a BOLT11 invoice the server needs before paying.

use std::str::FromStr;

use lightning_invoice::Bolt11Invoice;

/// What we know about an invoice before paying it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceInfo {
    /// Normalized invoice string (lowercase, no `lightning:` prefix).
    pub invoice: String,
    /// Amount encoded in the invoice, if any.
    pub amount_msat: Option<u64>,
    /// Hex payment hash.
    pub payment_hash: String,
    /// `true` when the invoice expiry has passed.
    pub expired: bool,
    /// Human description, when present.
    pub description: Option<String>,
}

/// Decode a BOLT11 invoice, accepting an optional `lightning:` prefix.
pub fn inspect(raw: &str) -> Result<InvoiceInfo, String> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("lightning:")
        .or_else(|| trimmed.strip_prefix("LIGHTNING:"))
        .unwrap_or(trimmed);
    let invoice = without_scheme.to_ascii_lowercase();
    let parsed =
        Bolt11Invoice::from_str(&invoice).map_err(|e| format!("invalid BOLT11 invoice: {e}"))?;
    let description = match parsed.description() {
        lightning_invoice::Bolt11InvoiceDescriptionRef::Direct(d) => {
            let text = d.to_string();
            (!text.is_empty()).then_some(text)
        }
        lightning_invoice::Bolt11InvoiceDescriptionRef::Hash(_) => None,
    };
    Ok(InvoiceInfo {
        amount_msat: parsed.amount_milli_satoshis(),
        payment_hash: hex::encode(parsed.payment_hash().as_ref() as &[u8]),
        expired: parsed.is_expired(),
        description,
        invoice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // BOLT11 spec test vector: "Please send $3 for a cup of coffee", 2500u.
    const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";

    #[test]
    fn decodes_amount_hash_and_description() {
        let info = inspect(&format!("lightning:{}", COFFEE.to_uppercase())).unwrap();
        assert_eq!(info.amount_msat, Some(250_000_000));
        assert_eq!(
            info.payment_hash,
            "0001020304050607080900010203040506070809000102030405060708090102"
        );
        assert_eq!(info.description.as_deref(), Some("1 cup coffee"));
        assert!(info.expired, "spec vector is from 2017");
        assert_eq!(info.invoice, COFFEE);
    }

    #[test]
    fn rejects_garbage() {
        assert!(inspect("lnbc1notaninvoice").is_err());
        assert!(inspect("").is_err());
    }
}
