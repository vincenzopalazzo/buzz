//! Payment logic behind the MCP tools, independent of MCP plumbing.
//!
//! Safety rules, all enforced here:
//! - a per-payment cap and a per-process budget, reserved before paying;
//! - an invoice is paid at most once per process, and a repeat call returns
//!   the first receipt instead of paying again;
//! - a timeout is an *unknown* outcome: the budget stays reserved and the
//!   caller must use `check_payment`, never pay again;
//! - a Sonar receipt is produced only after the wallet reports settlement.

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::bolt11;
use crate::receipt;
use crate::wallet::{LookupState, Paid, Wallet, WalletError};

/// Spending limits for one server process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest single payment, in msat.
    pub max_payment_msat: u64,
    /// Total that may be spent (settled or in flight) by this process, in msat.
    pub budget_msat: u64,
}

/// What a tool hands back to the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// Human/LLM-facing text, including the receipt lines when there are any.
    pub text: String,
    /// Machine-readable details.
    pub json: serde_json::Value,
    /// `true` when the tool failed.
    pub is_error: bool,
}

impl Reply {
    fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            json: serde_json::json!({ "status": "error", "error": message }),
            text: message,
            is_error: true,
        }
    }
}

#[derive(Debug, Clone)]
enum Attempt {
    InFlight { amount_msat: u64 },
    Unknown { amount_msat: u64 },
    Paid(Settled),
}

#[derive(Debug, Clone)]
struct Settled {
    receipt_id: String,
    amount_msat: u64,
    fee_msat: Option<u64>,
    preimage: Option<String>,
}

#[derive(Default)]
struct State {
    /// Settled plus in-flight/unknown spend, in msat.
    committed_msat: u64,
    attempts: HashMap<String, Attempt>,
}

/// Payment desk over a [`Wallet`].
pub struct Desk<W> {
    wallet: W,
    limits: Limits,
    state: Mutex<State>,
}

fn sats_text(msat: u64) -> String {
    if msat.is_multiple_of(1000) {
        format!("{} sats", msat / 1000)
    } else {
        format!("{}.{:03} sats", msat / 1000, msat % 1000)
    }
}

/// `Some(preimage)` only if it is 64 hex chars and hashes to `payment_hash`.
fn verified_preimage(preimage: &str, payment_hash: &str) -> Option<String> {
    let p = receipt::normalize_preimage(preimage)?;
    let bytes = hex::decode(&p).ok()?;
    (hex::encode(Sha256::digest(&bytes)) == payment_hash).then_some(p)
}

fn paid_reply(payment_hash: &str, s: &Settled, repeat: bool) -> Reply {
    let sats = receipt::bubble_sats(s.amount_msat);
    let lines = format!(
        "{}\n{}",
        receipt::pay_line(&s.receipt_id, sats),
        receipt::done_line(&s.receipt_id, s.preimage.as_deref())
    );
    let fee = s
        .fee_msat
        .map(|f| format!(" (fee {})", sats_text(f)))
        .unwrap_or_default();
    let lead = if repeat {
        "This invoice was already paid; no new payment was made."
    } else {
        "Payment settled."
    };
    let text = format!(
        "{lead} Paid {}{fee}. Payment hash {payment_hash}.\n\n\
         Chat receipt: put these two lines in your chat reply exactly as written, \
         each on its own line and outside any code block. Buzz and Sonar show them \
         as a payment bubble; do not reformat, translate or explain them.\n\
         {lines}",
        sats_text(s.amount_msat)
    );
    Reply {
        json: serde_json::json!({
            "status": "paid",
            "amount_sats": sats,
            "amount_msat": s.amount_msat,
            "fee_msat": s.fee_msat,
            "payment_hash": payment_hash,
            "preimage": s.preimage,
            "receipt_id": s.receipt_id,
            "chat_receipt": lines,
            "already_paid": repeat,
        }),
        text,
        is_error: false,
    }
}

impl<W: Wallet> Desk<W> {
    /// New desk with these limits.
    pub fn new(wallet: W, limits: Limits) -> Self {
        Self {
            wallet,
            limits,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Pay a BOLT11 invoice. `amount_sats` is required for amountless
    /// invoices and must match the invoice amount otherwise.
    pub async fn pay_invoice(&self, invoice: &str, amount_sats: Option<u64>) -> Reply {
        let info = match bolt11::inspect(invoice) {
            Ok(info) => info,
            Err(e) => return Reply::error(e),
        };
        if info.expired {
            return Reply::error("this invoice has expired; ask for a new one");
        }
        let requested = amount_sats.map(|s| s.saturating_mul(1000));
        let amount_msat = match (info.amount_msat, requested) {
            (Some(a), None) => a,
            (Some(a), Some(r)) if a == r => a,
            (Some(a), Some(_)) => {
                return Reply::error(format!(
                    "the invoice asks for {}; amount_sats does not match",
                    sats_text(a)
                ))
            }
            (None, Some(r)) => r,
            (None, None) => return Reply::error("this invoice has no amount; pass amount_sats"),
        };
        if amount_msat == 0 {
            return Reply::error("amount must be greater than zero");
        }
        let hash = info.payment_hash.clone();

        // Reserve under one lock: duplicate check, limits, budget.
        {
            let mut st = self.state();
            match st.attempts.get(&hash) {
                Some(Attempt::Paid(s)) => return paid_reply(&hash, s, true),
                Some(Attempt::InFlight { .. }) | Some(Attempt::Unknown { .. }) => {
                    return Reply::error(format!(
                        "a payment for this invoice is already in flight or its outcome is \
                         unknown; do not pay again, call check_payment with payment_hash {hash}"
                    ))
                }
                None => {}
            }
            if amount_msat > self.limits.max_payment_msat {
                return Reply::error(format!(
                    "{} exceeds the per-payment limit of {}",
                    sats_text(amount_msat),
                    sats_text(self.limits.max_payment_msat)
                ));
            }
            let remaining = self.limits.budget_msat.saturating_sub(st.committed_msat);
            if amount_msat > remaining {
                return Reply::error(format!(
                    "{} exceeds the remaining session budget of {}",
                    sats_text(amount_msat),
                    sats_text(remaining)
                ));
            }
            st.committed_msat += amount_msat;
            st.attempts
                .insert(hash.clone(), Attempt::InFlight { amount_msat });
        }

        let invoice_amount = if info.amount_msat.is_some() {
            None
        } else {
            Some(amount_msat)
        };
        let result = self.wallet.pay_invoice(&info.invoice, invoice_amount).await;
        let mut st = self.state();
        match result {
            Ok(Paid { preimage, fee_msat }) => {
                let settled = Settled {
                    receipt_id: receipt::new_receipt_id(),
                    amount_msat,
                    fee_msat,
                    preimage: verified_preimage(&preimage, &hash),
                };
                st.attempts
                    .insert(hash.clone(), Attempt::Paid(settled.clone()));
                paid_reply(&hash, &settled, false)
            }
            Err(WalletError::Timeout) => {
                st.attempts
                    .insert(hash.clone(), Attempt::Unknown { amount_msat });
                let mut reply = Reply::error(format!(
                    "The wallet did not confirm in time, so the outcome is unknown and the \
                     payment may still settle. Do not pay again. Call check_payment with \
                     payment_hash {hash} to learn the result; it returns the chat receipt \
                     once settled."
                ));
                reply.json = serde_json::json!({
                    "status": "unknown",
                    "payment_hash": hash,
                    "amount_msat": amount_msat,
                });
                reply
            }
            Err(e) => {
                st.attempts.remove(&hash);
                st.committed_msat = st.committed_msat.saturating_sub(amount_msat);
                Reply::error(format!("Payment failed, nothing was sent: {e}"))
            }
        }
    }

    /// Look a payment up by hash. Returns the chat receipt for a settled
    /// outgoing payment, the same one every time.
    pub async fn check_payment(&self, payment_hash: &str) -> Reply {
        let hash = payment_hash.trim().to_ascii_lowercase();
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Reply::error("payment_hash must be 64 hex characters");
        }
        if let Some(Attempt::Paid(s)) = self.state().attempts.get(&hash) {
            return paid_reply(&hash, s, true);
        }
        let state = match self.wallet.lookup(&hash).await {
            Ok(s) => s,
            Err(e) => return Reply::error(format!("lookup failed: {e}")),
        };
        let mut st = self.state();
        let ours = st.attempts.get(&hash).cloned();
        match state {
            LookupState::Settled {
                preimage,
                amount_msat,
                fee_msat,
                outgoing,
            } if outgoing || ours.is_some() => {
                let amount_msat = match ours {
                    Some(Attempt::InFlight { amount_msat } | Attempt::Unknown { amount_msat }) => {
                        amount_msat
                    }
                    _ => amount_msat,
                };
                let settled = Settled {
                    receipt_id: receipt::new_receipt_id(),
                    amount_msat,
                    fee_msat: Some(fee_msat),
                    preimage: preimage.and_then(|p| verified_preimage(&p, &hash)),
                };
                st.attempts
                    .insert(hash.clone(), Attempt::Paid(settled.clone()));
                let mut reply = paid_reply(&hash, &settled, false);
                reply.text = reply
                    .text
                    .replacen("Payment settled.", "The payment settled.", 1);
                reply
            }
            LookupState::Settled { amount_msat, .. } => Reply {
                text: format!("Received {} on invoice {hash}.", sats_text(amount_msat)),
                json: serde_json::json!({
                    "status": "received", "payment_hash": hash, "amount_msat": amount_msat,
                }),
                is_error: false,
            },
            LookupState::Failed => {
                if let Some(Attempt::InFlight { amount_msat } | Attempt::Unknown { amount_msat }) =
                    ours
                {
                    st.attempts.remove(&hash);
                    st.committed_msat = st.committed_msat.saturating_sub(amount_msat);
                }
                Reply {
                    text: format!("Payment {hash} failed or expired; nothing was sent."),
                    json: serde_json::json!({ "status": "failed", "payment_hash": hash }),
                    is_error: false,
                }
            }
            LookupState::Pending => Reply {
                text: format!(
                    "Payment {hash} is still pending. Check again later; do not pay again."
                ),
                json: serde_json::json!({ "status": "pending", "payment_hash": hash }),
                is_error: false,
            },
            LookupState::NotFound => Reply {
                text: format!("The wallet has no record of payment {hash}."),
                json: serde_json::json!({ "status": "not_found", "payment_hash": hash }),
                is_error: false,
            },
        }
    }

    /// Create an invoice to receive `amount_sats`.
    pub async fn create_invoice(
        &self,
        amount_sats: u64,
        description: Option<String>,
        expiry_secs: Option<u64>,
    ) -> Reply {
        if amount_sats == 0 {
            return Reply::error("amount_sats must be greater than zero");
        }
        match self
            .wallet
            .make_invoice(amount_sats.saturating_mul(1000), description, expiry_secs)
            .await
        {
            Ok(inv) => Reply {
                text: format!(
                    "Invoice for {amount_sats} sats:\n{}\n\nShare it with the payer. \
                     Use check_payment with payment_hash {} to see when it is paid.",
                    inv.bolt11,
                    inv.payment_hash
                        .as_deref()
                        .unwrap_or("(not reported by the wallet)")
                ),
                json: serde_json::json!({
                    "status": "created",
                    "invoice": inv.bolt11,
                    "payment_hash": inv.payment_hash,
                    "amount_sats": amount_sats,
                }),
                is_error: false,
            },
            Err(e) => Reply::error(format!("could not create the invoice: {e}")),
        }
    }

    /// Wallet balance plus what this session may still spend.
    pub async fn balance(&self) -> Reply {
        let remaining = self
            .limits
            .budget_msat
            .saturating_sub(self.state().committed_msat);
        match self.wallet.balance_msat().await {
            Ok(b) => Reply {
                text: format!(
                    "Wallet balance {}. This session may still spend {} (max {} per payment).",
                    sats_text(b),
                    sats_text(remaining),
                    sats_text(self.limits.max_payment_msat)
                ),
                json: serde_json::json!({
                    "balance_msat": b,
                    "session_remaining_msat": remaining,
                    "max_payment_msat": self.limits.max_payment_msat,
                }),
                is_error: false,
            },
            Err(e) => Reply::error(format!("could not read the balance: {e}")),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_invoices {
    use bitcoin::hashes::{sha256, Hash};
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};

    /// Preimage of 32 zero bytes and its hash.
    pub const PREIMAGE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    pub const HASH: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

    /// A signed, unexpired mainnet invoice for `HASH`.
    pub fn invoice(amount_msat: Option<u64>) -> String {
        let secp = Secp256k1::new();
        let key = SecretKey::from_slice(&[7u8; 32]).expect("valid key");
        let hash = sha256::Hash::from_slice(&hex::decode(HASH).expect("hex")).expect("32 bytes");
        let builder = InvoiceBuilder::new(Currency::Bitcoin)
            .description("buzz test".into())
            .payment_hash(hash)
            .payment_secret(PaymentSecret([42u8; 32]))
            .current_timestamp()
            .min_final_cltv_expiry_delta(144);
        let signed = match amount_msat {
            Some(a) => builder
                .amount_milli_satoshis(a)
                .build_signed(|h| secp.sign_ecdsa_recoverable(h, &key)),
            None => builder.build_signed(|h| secp.sign_ecdsa_recoverable(h, &key)),
        };
        signed.expect("signed invoice").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::test_invoices::{invoice, HASH, PREIMAGE};
    use super::*;
    use crate::wallet::fake::FakeWallet;

    const LIMITS: Limits = Limits {
        max_payment_msat: 10_000_000,
        budget_msat: 25_000_000,
    };

    fn desk(pay: Result<Paid, WalletError>) -> Desk<FakeWallet> {
        let wallet = FakeWallet::default();
        *wallet.pay.lock().unwrap() = Some(pay);
        Desk::new(wallet, LIMITS)
    }

    fn paid() -> Result<Paid, WalletError> {
        Ok(Paid {
            preimage: PREIMAGE.to_uppercase(),
            fee_msat: Some(1_000),
        })
    }

    fn receipt_lines(reply: &Reply) -> Vec<String> {
        reply
            .text
            .lines()
            .filter(|l| l.starts_with('⚡'))
            .map(str::to_string)
            .collect()
    }

    #[tokio::test]
    async fn settled_payment_returns_sonar_receipt() {
        let d = desk(paid());
        let reply = d.pay_invoice(&invoice(Some(21_000)), None).await;
        assert!(!reply.is_error, "{}", reply.text);
        let lines = receipt_lines(&reply);
        assert_eq!(lines.len(), 2);
        let id = reply.json["receipt_id"].as_str().unwrap();
        assert_eq!(lines[0], format!("⚡PAY|1|{id}|21"));
        assert_eq!(lines[1], format!("⚡PAYDONE|2|{id}|{PREIMAGE}"));
        assert_eq!(reply.json["payment_hash"], HASH);
        assert_eq!(reply.json["chat_receipt"], lines.join("\n"));
    }

    #[tokio::test]
    async fn same_invoice_is_never_paid_twice() {
        let d = desk(paid());
        let inv = invoice(Some(21_000));
        let first = d.pay_invoice(&inv, None).await;
        let second = d.pay_invoice(&inv, None).await;
        assert_eq!(d.wallet.pay_calls.lock().unwrap().len(), 1);
        assert_eq!(second.json["already_paid"], true);
        assert_eq!(receipt_lines(&first), receipt_lines(&second));
    }

    #[tokio::test]
    async fn wrong_preimage_is_dropped_from_the_receipt() {
        let d = desk(Ok(Paid {
            preimage: "11".repeat(32),
            fee_msat: None,
        }));
        let reply = d.pay_invoice(&invoice(Some(1_000)), None).await;
        let lines = receipt_lines(&reply);
        let id = reply.json["receipt_id"].as_str().unwrap();
        assert_eq!(lines[1], format!("⚡PAYDONE|2|{id}"));
    }

    #[tokio::test]
    async fn limits_and_budget_are_enforced_before_paying() {
        let d = desk(paid());
        let too_big = d.pay_invoice(&invoice(Some(10_000_001)), None).await;
        assert!(too_big.is_error && too_big.text.contains("per-payment limit"));
        assert!(d.wallet.pay_calls.lock().unwrap().is_empty());

        let tight = Desk::new(
            FakeWallet::default(),
            Limits {
                max_payment_msat: 10_000_000,
                budget_msat: 5_000,
            },
        );
        let over = tight.pay_invoice(&invoice(Some(6_000)), None).await;
        assert!(over.is_error && over.text.contains("session budget"));
    }

    #[tokio::test]
    async fn amountless_invoices_need_a_matching_amount() {
        let d = desk(paid());
        let missing = d.pay_invoice(&invoice(None), None).await;
        assert!(missing.is_error && missing.text.contains("no amount"));
        let ok = d.pay_invoice(&invoice(None), Some(5)).await;
        assert!(!ok.is_error, "{}", ok.text);
        assert_eq!(
            d.wallet.pay_calls.lock().unwrap()[0].1,
            Some(5_000),
            "amount goes to the wallet only for amountless invoices"
        );

        let d = desk(paid());
        let mismatch = d.pay_invoice(&invoice(Some(21_000)), Some(22)).await;
        assert!(mismatch.is_error && mismatch.text.contains("does not match"));
    }

    #[tokio::test]
    async fn failure_releases_budget_timeout_does_not() {
        let d = desk(Err(WalletError::Rejected {
            code: "INSUFFICIENT_BALANCE".into(),
            message: "no funds".into(),
        }));
        let failed = d.pay_invoice(&invoice(Some(21_000)), None).await;
        assert!(failed.is_error && failed.text.contains("nothing was sent"));
        assert_eq!(d.state().committed_msat, 0);

        let d = desk(Err(WalletError::Timeout));
        let unknown = d.pay_invoice(&invoice(Some(21_000)), None).await;
        assert_eq!(unknown.json["status"], "unknown");
        assert!(
            receipt_lines(&unknown).is_empty(),
            "no receipt before settlement"
        );
        assert_eq!(d.state().committed_msat, 21_000);
        let retry = d.pay_invoice(&invoice(Some(21_000)), None).await;
        assert!(retry.is_error && retry.text.contains("do not pay again"));
        assert_eq!(d.wallet.pay_calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn check_payment_turns_an_unknown_into_a_receipt() {
        let d = desk(Err(WalletError::Timeout));
        d.pay_invoice(&invoice(Some(21_000)), None).await;
        *d.wallet.lookup.lock().unwrap() = Some(Ok(LookupState::Settled {
            preimage: Some(PREIMAGE.into()),
            amount_msat: 21_000,
            fee_msat: 0,
            outgoing: true,
        }));
        let settled = d.check_payment(HASH).await;
        let lines = receipt_lines(&settled);
        assert_eq!(lines.len(), 2, "{}", settled.text);
        assert!(lines[0].ends_with("|21"));
        // Stable: asking again returns the same receipt id.
        assert_eq!(receipt_lines(&d.check_payment(HASH).await), lines);
    }

    #[tokio::test]
    async fn check_payment_failure_releases_the_reservation() {
        let d = desk(Err(WalletError::Timeout));
        d.pay_invoice(&invoice(Some(21_000)), None).await;
        *d.wallet.lookup.lock().unwrap() = Some(Ok(LookupState::Failed));
        let failed = d.check_payment(HASH).await;
        assert_eq!(failed.json["status"], "failed");
        assert_eq!(d.state().committed_msat, 0);
    }

    #[tokio::test]
    async fn rejects_expired_and_garbage_invoices() {
        let d = desk(paid());
        let expired = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";
        assert!(d.pay_invoice(expired, None).await.text.contains("expired"));
        assert!(d.pay_invoice("hello", None).await.is_error);
        assert!(d.wallet.pay_calls.lock().unwrap().is_empty());
    }
}
