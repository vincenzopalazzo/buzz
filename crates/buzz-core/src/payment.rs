//! Pure Lightning payment domain for NIP-LP (`docs/nips/NIP-LP.md`).
//!
//! Tag validation for payment requests (kind 40009) and payment receipts
//! (kind 40010), preimage verification, and the client-side state machine
//! that turns a request plus its receipts into a renderable card state.
//!
//! No network I/O and no bolt11/bolt12 decoding: invoices and offers are
//! opaque strings here. A wallet (an MCP server, a CLI, a node) decodes and
//! pays; Buzz only carries the request and the result.
//!
//! The shape of this module follows the earlier NWC wallet attempt
//! (block/buzz#2635 by Marco Pesani); the wallet itself was dropped so the
//! wire format can land on its own.

use sha2::{Digest, Sha256};
use std::str::FromStr;
use thiserror::Error;

/// Tag name carrying the channel id on both kinds.
pub const TAG_CHANNEL: &str = "h";
/// Tag name carrying the payee pubkey on a request.
pub const TAG_PAYEE: &str = "p";
/// Tag name carrying the amount in millisatoshis.
pub const TAG_AMOUNT: &str = "amount";
/// Tag name carrying a BOLT11 invoice.
pub const TAG_BOLT11: &str = "bolt11";
/// Tag name carrying a BOLT12 offer.
pub const TAG_BOLT12: &str = "bolt12";
/// Tag name carrying a LUD-16 Lightning Address.
pub const TAG_LUD16: &str = "lud16";
/// Tag name carrying a BIP-353 human-readable address.
pub const TAG_BIP353: &str = "bip353";
/// Tag name carrying the hex payment hash.
pub const TAG_PAYMENT_HASH: &str = "payment_hash";
/// Tag name carrying the hex preimage on a receipt.
pub const TAG_PREIMAGE: &str = "preimage";
/// Tag name carrying the optional memo on a request.
pub const TAG_MEMO: &str = "memo";
/// Tag name carrying the unix-seconds expiry on a request.
///
/// Deliberately `expiry`, never NIP-40 `expiration`: a relay implementing
/// NIP-40 auto-delete would otherwise sweep pay cards out of history.
pub const TAG_EXPIRY: &str = "expiry";
/// Tag name carrying the receipt outcome (`paid` / `failed`).
pub const TAG_STATUS: &str = "status";
/// Tag name carrying the routing fee paid, in millisatoshis.
pub const TAG_FEE: &str = "fee";
/// Tag name carrying a short failure reason on a failed receipt.
pub const TAG_REASON: &str = "reason";

/// Maximum accepted length of a memo or failure reason, in bytes.
pub const MAX_TEXT_BYTES: usize = 512;

/// Millisatoshis. The domain speaks msat everywhere, event tags included.
///
/// Sats exist only at the UI boundary. No floating-point amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Amount(u64);

impl Amount {
    /// Construct from a millisatoshi count.
    pub fn from_msat(msat: u64) -> Self {
        Self(msat)
    }

    /// Construct from whole satoshis.
    pub fn from_sat(sat: u64) -> Self {
        Self(sat.saturating_mul(1000))
    }

    /// Millisatoshis represented by this amount.
    pub fn as_msat(self) -> u64 {
        self.0
    }

    /// Whole satoshis, truncating any sub-satoshi remainder.
    pub fn as_sat_floor(self) -> u64 {
        self.0 / 1000
    }

    /// Parse an amount tag value: ASCII decimal digits only.
    ///
    /// Rejects empty strings, signs, floats, and values that overflow `u64`.
    pub fn parse_tag(s: &str) -> Result<Self, PaymentError> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(PaymentError::MalformedAmount);
        }
        let msat = u64::from_str(s).map_err(|_| PaymentError::MalformedAmount)?;
        Ok(Self(msat))
    }

    /// Render this amount as a decimal tag string.
    pub fn to_tag_string(self) -> String {
        self.0.to_string()
    }
}

/// Returns `true` iff `SHA256(preimage) == payment_hash`.
///
/// Inputs are hex strings as they appear in event tags. Malformed hex or a
/// length other than 32 bytes on either side yields `false`.
pub fn verify_preimage(preimage: &str, payment_hash: &str) -> bool {
    let Ok(preimage_bytes) = hex::decode(preimage) else {
        return false;
    };
    let Ok(hash_bytes) = hex::decode(payment_hash) else {
        return false;
    };
    if preimage_bytes.len() != 32 || hash_bytes.len() != 32 {
        return false;
    }
    Sha256::digest(&preimage_bytes).as_slice() == hash_bytes.as_slice()
}

/// One way a payment request can be paid.
///
/// A request carries at least one target. Payers pick whichever their wallet
/// supports; a `bolt11` is the only target whose `payment_hash` is known to
/// the payee up front and therefore the only one whose receipt can be bound
/// to the request (see [`PaymentState::Paid`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentTarget {
    /// A BOLT11 invoice (opaque string, not decoded here).
    Bolt11(String),
    /// A BOLT12 offer (`lno1…`, opaque string, not decoded here).
    Bolt12(String),
    /// A LUD-16 Lightning Address (`user@domain`).
    Lud16(String),
    /// A BIP-353 human-readable address (`user@domain`, optional `₿` prefix).
    Bip353(String),
}

impl PaymentTarget {
    /// Tag name this target is carried in.
    pub fn tag_name(&self) -> &'static str {
        match self {
            PaymentTarget::Bolt11(_) => TAG_BOLT11,
            PaymentTarget::Bolt12(_) => TAG_BOLT12,
            PaymentTarget::Lud16(_) => TAG_LUD16,
            PaymentTarget::Bip353(_) => TAG_BIP353,
        }
    }

    /// The opaque target string.
    pub fn value(&self) -> &str {
        match self {
            PaymentTarget::Bolt11(s)
            | PaymentTarget::Bolt12(s)
            | PaymentTarget::Lud16(s)
            | PaymentTarget::Bip353(s) => s,
        }
    }
}

/// A validated kind-40009 payment request (tag shape only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentRequest {
    /// Amount in millisatoshis (strictly positive).
    pub amount: Amount,
    /// Pay targets, in tag order. Never empty.
    pub targets: Vec<PaymentTarget>,
    /// Channel id from the required `h` tag.
    pub channel_id: String,
    /// Payee pubkey from the required `p` tag.
    pub payee_pubkey: String,
    /// Hex payment hash, when the payee minted the invoice and published it.
    pub payment_hash: Option<String>,
    /// Optional human-readable memo.
    pub memo: Option<String>,
    /// Optional unix-seconds expiry.
    pub expiry: Option<u64>,
}

/// Outcome reported by a kind-40010 receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptStatus {
    /// The payer's wallet reported the payment settled.
    Paid,
    /// The payer's wallet reported the payment failed.
    Failed,
}

impl ReceiptStatus {
    /// Tag value for this status.
    pub fn as_tag_value(self) -> &'static str {
        match self {
            ReceiptStatus::Paid => "paid",
            ReceiptStatus::Failed => "failed",
        }
    }

    /// Parse a `status` tag value.
    pub fn parse_tag(s: &str) -> Result<Self, PaymentError> {
        match s {
            "paid" => Ok(ReceiptStatus::Paid),
            "failed" => Ok(ReceiptStatus::Failed),
            _ => Err(PaymentError::MalformedTag(TAG_STATUS)),
        }
    }
}

/// A validated kind-40010 payment receipt (tag shape only).
///
/// Shape validation does not imply the payment settled. A receipt is a claim
/// by its author; [`verified`](PaymentReceipt::verified) upgrades it to a
/// proof only when a preimage is present and matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentReceipt {
    /// Channel id from the required `h` tag.
    pub channel_id: String,
    /// Request event id from the bare `e` tag.
    pub request_id: String,
    /// Reported outcome. Absent `status` tag means `paid`.
    pub status: ReceiptStatus,
    /// Amount the payer reports sending, in msat.
    pub amount: Amount,
    /// Hex payment hash. Required on `paid` receipts.
    pub payment_hash: Option<String>,
    /// Hex preimage, when the wallet returned one.
    pub preimage: Option<String>,
    /// Routing fee in msat, when reported.
    pub fee: Option<Amount>,
    /// Short failure reason on `failed` receipts.
    pub reason: Option<String>,
}

impl PaymentReceipt {
    /// `true` iff this receipt carries a preimage that hashes to its
    /// `payment_hash`. Says nothing about whether that hash belongs to the
    /// request; see [`payment_state`].
    pub fn verified(&self) -> bool {
        match (&self.preimage, &self.payment_hash) {
            (Some(p), Some(h)) => verify_preimage(p, h),
            _ => false,
        }
    }
}

/// Parse/validation failures for payment request and receipt tags.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PaymentError {
    /// A required tag name is absent.
    #[error("missing required tag: {0}")]
    MissingTag(&'static str),
    /// A present tag value failed structural parsing.
    #[error("malformed tag: {0}")]
    MalformedTag(&'static str),
    /// An amount tag was not a strict decimal integer string.
    #[error("malformed amount")]
    MalformedAmount,
    /// Amount was present but zero.
    #[error("amount must be greater than zero")]
    ZeroAmount,
    /// None of `bolt11`, `bolt12`, `lud16`, `bip353` was present.
    #[error("payment request requires at least one of bolt11, bolt12, lud16, bip353")]
    MissingPaymentTarget,
    /// A memo or reason exceeded [`MAX_TEXT_BYTES`].
    #[error("tag text too long: {0}")]
    TextTooLong(&'static str),
}

impl PaymentRequest {
    /// Parse and validate a kind-40009 tag set.
    ///
    /// Required: non-zero `amount`, `h`, `p`, and at least one target tag.
    /// Optional: `payment_hash` (64 hex), `memo`, `expiry` (unix seconds).
    pub fn from_tags(tags: &[Vec<String>]) -> Result<Self, PaymentError> {
        let amount = match first_tag(tags, TAG_AMOUNT) {
            None => return Err(PaymentError::MissingTag(TAG_AMOUNT)),
            Some(raw) => Amount::parse_tag(raw)?,
        };
        if amount.as_msat() == 0 {
            return Err(PaymentError::ZeroAmount);
        }
        let channel_id = required_nonempty(tags, TAG_CHANNEL)?.to_string();
        let payee_pubkey = required_hex(tags, TAG_PAYEE, 64)?.to_string();
        let targets = payment_targets(tags)?;
        let payment_hash = optional_hex(tags, TAG_PAYMENT_HASH, 64)?.map(str::to_string);
        let memo = optional_text(tags, TAG_MEMO)?.map(str::to_string);
        let expiry = match first_tag(tags, TAG_EXPIRY) {
            None => None,
            Some(raw) => Some(parse_unix_seconds(raw, TAG_EXPIRY)?),
        };
        Ok(Self {
            amount,
            targets,
            channel_id,
            payee_pubkey,
            payment_hash,
            memo,
            expiry,
        })
    }

    /// `true` iff the request carries an `expiry` at or before `now`.
    pub fn is_expired_at(&self, now: u64) -> bool {
        self.expiry.is_some_and(|t| t <= now)
    }
}

impl PaymentReceipt {
    /// Parse and validate a kind-40010 tag set (shape only).
    ///
    /// Required: `h`, a bare `["e", "<id>"]`, `amount`. `status` defaults to
    /// `paid`; a `paid` receipt must carry `payment_hash`. `preimage`, `fee`
    /// and `reason` are optional.
    pub fn from_tags(tags: &[Vec<String>]) -> Result<Self, PaymentError> {
        let channel_id = required_nonempty(tags, TAG_CHANNEL)?.to_string();
        let request_id = bare_e_tag(tags)?.to_string();
        let status = match first_tag(tags, TAG_STATUS) {
            None => ReceiptStatus::Paid,
            Some(raw) => ReceiptStatus::parse_tag(raw)?,
        };
        let amount = match first_tag(tags, TAG_AMOUNT) {
            None => return Err(PaymentError::MissingTag(TAG_AMOUNT)),
            Some(raw) => Amount::parse_tag(raw)?,
        };
        let payment_hash = optional_hex(tags, TAG_PAYMENT_HASH, 64)?.map(str::to_string);
        if status == ReceiptStatus::Paid && payment_hash.is_none() {
            return Err(PaymentError::MissingTag(TAG_PAYMENT_HASH));
        }
        let preimage = optional_hex(tags, TAG_PREIMAGE, 64)?.map(str::to_string);
        let fee = match first_tag(tags, TAG_FEE) {
            None => None,
            Some(raw) => Some(Amount::parse_tag(raw)?),
        };
        let reason = optional_text(tags, TAG_REASON)?.map(str::to_string);
        Ok(Self {
            channel_id,
            request_id,
            status,
            amount,
            payment_hash,
            preimage,
            fee,
            reason,
        })
    }
}

/// Renderable state of a request given the receipts that reference it.
///
/// Derived purely from event data plus a caller-supplied clock, so every
/// client (desktop, mobile, CLI, an agent reading JSON) reaches the same
/// answer without a wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentState {
    /// No settled receipt yet and the request has not expired.
    Pending,
    /// No settled receipt and `expiry` has passed.
    Expired,
    /// The most recent receipt reports a failure; the request is still open.
    Failed,
    /// At least one receipt reports settlement.
    Paid {
        /// `true` iff some `paid` receipt carries a preimage matching its
        /// hash **and** that hash is the request's own `payment_hash`.
        /// Without a hash on the request the receipt stays a claim.
        verified: bool,
    },
}

/// Fold a request and its receipts (any order) into a [`PaymentState`].
///
/// `receipts` should already be filtered to those whose `request_id` is the
/// request's event id; receipts for other requests are ignored only by
/// `request_id` mismatch when `request_id` is supplied.
pub fn payment_state<'a>(
    request: &PaymentRequest,
    request_id: Option<&str>,
    receipts: impl IntoIterator<Item = &'a PaymentReceipt>,
    now: u64,
) -> PaymentState {
    let mut saw_paid = false;
    let mut verified = false;
    let mut saw_failed = false;
    for r in receipts {
        if request_id.is_some_and(|id| id != r.request_id) {
            continue;
        }
        match r.status {
            ReceiptStatus::Paid => {
                saw_paid = true;
                let bound = match (&request.payment_hash, &r.payment_hash) {
                    (Some(want), Some(got)) => want == got,
                    _ => false,
                };
                if bound && r.verified() {
                    verified = true;
                }
            }
            ReceiptStatus::Failed => saw_failed = true,
        }
    }
    if saw_paid {
        PaymentState::Paid { verified }
    } else if request.is_expired_at(now) {
        PaymentState::Expired
    } else if saw_failed {
        PaymentState::Failed
    } else {
        PaymentState::Pending
    }
}

fn first_tag<'a>(tags: &'a [Vec<String>], name: &str) -> Option<&'a str> {
    tags.iter()
        .find(|t| t.first().map(String::as_str) == Some(name))
        .and_then(|t| t.get(1).map(String::as_str))
}

fn required_nonempty<'a>(
    tags: &'a [Vec<String>],
    name: &'static str,
) -> Result<&'a str, PaymentError> {
    match first_tag(tags, name) {
        Some(v) if !v.is_empty() => Ok(v),
        Some(_) => Err(PaymentError::MalformedTag(name)),
        None => Err(PaymentError::MissingTag(name)),
    }
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn required_hex<'a>(
    tags: &'a [Vec<String>],
    name: &'static str,
    len: usize,
) -> Result<&'a str, PaymentError> {
    let v = required_nonempty(tags, name)?;
    if is_lower_hex(v, len) {
        Ok(v)
    } else {
        Err(PaymentError::MalformedTag(name))
    }
}

fn optional_hex<'a>(
    tags: &'a [Vec<String>],
    name: &'static str,
    len: usize,
) -> Result<Option<&'a str>, PaymentError> {
    match first_tag(tags, name) {
        None => Ok(None),
        Some(v) if is_lower_hex(v, len) => Ok(Some(v)),
        Some(_) => Err(PaymentError::MalformedTag(name)),
    }
}

fn optional_text<'a>(
    tags: &'a [Vec<String>],
    name: &'static str,
) -> Result<Option<&'a str>, PaymentError> {
    let Some(v) = first_tag(tags, name) else {
        return Ok(None);
    };
    if v.is_empty() {
        return Ok(None);
    }
    if v.len() > MAX_TEXT_BYTES {
        return Err(PaymentError::TextTooLong(name));
    }
    if v.chars().any(char::is_control) {
        return Err(PaymentError::MalformedTag(name));
    }
    Ok(Some(v))
}

fn payment_targets(tags: &[Vec<String>]) -> Result<Vec<PaymentTarget>, PaymentError> {
    let mut out = Vec::new();
    for t in tags {
        let (Some(name), Some(value)) = (t.first(), t.get(1)) else {
            continue;
        };
        let value = value.trim();
        let target = match name.as_str() {
            TAG_BOLT11 => PaymentTarget::Bolt11(value.to_string()),
            TAG_BOLT12 => PaymentTarget::Bolt12(value.to_string()),
            TAG_LUD16 => PaymentTarget::Lud16(value.to_string()),
            TAG_BIP353 => PaymentTarget::Bip353(value.to_string()),
            _ => continue,
        };
        if value.is_empty() || value.chars().any(char::is_whitespace) {
            return Err(PaymentError::MalformedTag(target.tag_name()));
        }
        out.push(target);
    }
    if out.is_empty() {
        return Err(PaymentError::MissingPaymentTarget);
    }
    Ok(out)
}

fn bare_e_tag(tags: &[Vec<String>]) -> Result<&str, PaymentError> {
    let t = tags
        .iter()
        .find(|t| t.first().map(String::as_str) == Some("e"))
        .ok_or(PaymentError::MissingTag("e"))?;
    // Exactly ["e", "<id>"]: a marker or relay hint would make NIP-10
    // thread machinery treat the receipt as a reply.
    if t.len() != 2 || !is_lower_hex(&t[1], 64) {
        return Err(PaymentError::MalformedTag("e"));
    }
    Ok(&t[1])
}

fn parse_unix_seconds(raw: &str, name: &'static str) -> Result<u64, PaymentError> {
    if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit()) {
        return Err(PaymentError::MalformedTag(name));
    }
    u64::from_str(raw).map_err(|_| PaymentError::MalformedTag(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
    const PAYEE: &str = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    const REQUEST_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    // SHA256(0x00 * 32)
    const PREIMAGE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const HASH: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

    fn tags(pairs: &[(&str, &str)]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| vec![k.to_string(), v.to_string()])
            .collect()
    }

    fn request_tags() -> Vec<Vec<String>> {
        tags(&[
            ("h", CHANNEL),
            ("p", PAYEE),
            ("amount", "500000"),
            ("bolt11", "lnbc5u1pexample"),
            ("payment_hash", HASH),
            ("memo", "lunch"),
            ("expiry", "1700000000"),
        ])
    }

    fn receipt_tags(extra: &[(&str, &str)]) -> Vec<Vec<String>> {
        let mut base = vec![("h", CHANNEL), ("e", REQUEST_ID), ("amount", "500000")];
        base.extend_from_slice(extra);
        tags(&base)
    }

    #[test]
    fn amount_parse_rejects_non_digits() {
        assert!(Amount::parse_tag("").is_err());
        assert!(Amount::parse_tag("-1").is_err());
        assert!(Amount::parse_tag("1.5").is_err());
        assert!(Amount::parse_tag("99999999999999999999999").is_err());
        assert_eq!(Amount::parse_tag("001").unwrap().as_msat(), 1);
        assert_eq!(Amount::from_sat(21).as_msat(), 21_000);
        assert_eq!(Amount::from_msat(21_999).as_sat_floor(), 21);
    }

    #[test]
    fn verify_preimage_vectors() {
        assert!(verify_preimage(PREIMAGE, HASH));
        assert!(!verify_preimage(PREIMAGE, REQUEST_ID));
        assert!(!verify_preimage("zz", HASH));
        assert!(!verify_preimage("00", HASH));
    }

    #[test]
    fn request_parses_full_shape() {
        let r = PaymentRequest::from_tags(&request_tags()).unwrap();
        assert_eq!(r.amount.as_msat(), 500_000);
        assert_eq!(r.channel_id, CHANNEL);
        assert_eq!(r.payee_pubkey, PAYEE);
        assert_eq!(
            r.targets,
            vec![PaymentTarget::Bolt11("lnbc5u1pexample".into())]
        );
        assert_eq!(r.payment_hash.as_deref(), Some(HASH));
        assert_eq!(r.memo.as_deref(), Some("lunch"));
        assert_eq!(r.expiry, Some(1_700_000_000));
        assert!(r.is_expired_at(1_700_000_000));
        assert!(!r.is_expired_at(1_699_999_999));
    }

    #[test]
    fn request_accepts_every_target_kind_in_order() {
        let r = PaymentRequest::from_tags(&tags(&[
            ("h", CHANNEL),
            ("p", PAYEE),
            ("amount", "1"),
            ("bip353", "₿alice@example.com"),
            ("bolt12", "lno1example"),
            ("lud16", "alice@example.com"),
        ]))
        .unwrap();
        assert_eq!(r.targets.len(), 3);
        assert_eq!(r.targets[0].tag_name(), "bip353");
        assert_eq!(r.targets[1].value(), "lno1example");
        assert_eq!(r.targets[2].tag_name(), "lud16");
    }

    #[test]
    fn request_rejects_missing_or_bad_fields() {
        let mut t = request_tags();
        t.retain(|t| t[0] != "amount");
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MissingTag("amount"))
        );

        let mut t = request_tags();
        t[2][1] = "0".into();
        assert_eq!(PaymentRequest::from_tags(&t), Err(PaymentError::ZeroAmount));

        let mut t = request_tags();
        t.retain(|t| t[0] != "bolt11");
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MissingPaymentTarget)
        );

        let mut t = request_tags();
        t[1][1] = "ABCD".into();
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MalformedTag("p"))
        );

        let mut t = request_tags();
        t[4][1] = "nothex".into();
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MalformedTag("payment_hash"))
        );

        let mut t = request_tags();
        t[6][1] = "soon".into();
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MalformedTag("expiry"))
        );

        let mut t = request_tags();
        t[3][1] = "lnbc with space".into();
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::MalformedTag("bolt11"))
        );

        let mut t = request_tags();
        t[5][1] = "x".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            PaymentRequest::from_tags(&t),
            Err(PaymentError::TextTooLong("memo"))
        );
    }

    #[test]
    fn receipt_defaults_to_paid_and_needs_hash() {
        let r = PaymentReceipt::from_tags(&receipt_tags(&[("payment_hash", HASH)])).unwrap();
        assert_eq!(r.status, ReceiptStatus::Paid);
        assert_eq!(r.request_id, REQUEST_ID);
        assert!(!r.verified());

        assert_eq!(
            PaymentReceipt::from_tags(&receipt_tags(&[])),
            Err(PaymentError::MissingTag("payment_hash"))
        );
    }

    #[test]
    fn receipt_with_preimage_verifies() {
        let r = PaymentReceipt::from_tags(&receipt_tags(&[
            ("payment_hash", HASH),
            ("preimage", PREIMAGE),
            ("fee", "12"),
        ]))
        .unwrap();
        assert!(r.verified());
        assert_eq!(r.fee.map(Amount::as_msat), Some(12));

        let wrong = PaymentReceipt::from_tags(&receipt_tags(&[
            ("payment_hash", REQUEST_ID),
            ("preimage", PREIMAGE),
        ]))
        .unwrap();
        assert!(!wrong.verified());
    }

    #[test]
    fn failed_receipt_needs_no_hash() {
        let r = PaymentReceipt::from_tags(&receipt_tags(&[
            ("status", "failed"),
            ("reason", "no route"),
        ]))
        .unwrap();
        assert_eq!(r.status, ReceiptStatus::Failed);
        assert_eq!(r.reason.as_deref(), Some("no route"));
        assert_eq!(
            PaymentReceipt::from_tags(&receipt_tags(&[("status", "maybe")])),
            Err(PaymentError::MalformedTag("status"))
        );
    }

    #[test]
    fn receipt_rejects_marked_e_tag() {
        let mut t = receipt_tags(&[("payment_hash", HASH)]);
        t[1] = vec!["e".into(), REQUEST_ID.into(), "".into(), "reply".into()];
        assert_eq!(
            PaymentReceipt::from_tags(&t),
            Err(PaymentError::MalformedTag("e"))
        );
    }

    #[test]
    fn state_machine_orders_outcomes() {
        let req = PaymentRequest::from_tags(&request_tags()).unwrap();
        let none: Vec<PaymentReceipt> = vec![];
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), &none, 1_600_000_000),
            PaymentState::Pending
        );
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), &none, 1_700_000_000),
            PaymentState::Expired
        );

        let failed = PaymentReceipt::from_tags(&receipt_tags(&[("status", "failed")])).unwrap();
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), [&failed], 1_600_000_000),
            PaymentState::Failed
        );
        // Expiry beats a stale failure.
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), [&failed], 1_700_000_001),
            PaymentState::Expired
        );

        let claimed = PaymentReceipt::from_tags(&receipt_tags(&[("payment_hash", HASH)])).unwrap();
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), [&failed, &claimed], 1_800_000_000),
            PaymentState::Paid { verified: false }
        );

        let proven = PaymentReceipt::from_tags(&receipt_tags(&[
            ("payment_hash", HASH),
            ("preimage", PREIMAGE),
        ]))
        .unwrap();
        assert_eq!(
            payment_state(&req, Some(REQUEST_ID), [&claimed, &proven], 1_800_000_000),
            PaymentState::Paid { verified: true }
        );

        // A valid preimage for a hash the request never published is a claim.
        let mut unbound_req = req.clone();
        unbound_req.payment_hash = None;
        assert_eq!(
            payment_state(&unbound_req, Some(REQUEST_ID), [&proven], 1_600_000_000),
            PaymentState::Paid { verified: false }
        );

        // Receipts for another request are ignored.
        let other = "f".repeat(64);
        assert_eq!(
            payment_state(&req, Some(&other), [&proven], 1_600_000_000),
            PaymentState::Pending
        );
    }
}
