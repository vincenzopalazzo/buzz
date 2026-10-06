//! The wallet seam: a tiny trait over Nostr Wallet Connect (NIP-47).
//!
//! The server never holds keys or funds; every call goes to the user's own
//! wallet service (Alby Hub, LNbits, Phoenixd, CLN with an NWC plugin, ...).
//! [`FakeWallet`] backs the tests.

use std::time::Duration;

use nostr::nips::nip47::{
    Error as Nip47Error, ErrorCode, LookupInvoiceRequest, MakeInvoiceRequest,
    NostrWalletConnectURI, PayInvoiceRequest, TransactionState,
};
use nwc::{Error as NwcError, NostrWalletConnectOptions, NWC};

/// Why a wallet call did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WalletError {
    /// No reply in time. For a payment the outcome is **unknown**: it may
    /// still settle, so callers must look it up instead of paying again.
    #[error("the wallet did not answer in time; the outcome is unknown")]
    Timeout,
    /// The wallet answered with a NIP-47 error.
    #[error("wallet error ({code}): {message}")]
    Rejected {
        /// NIP-47 error code, e.g. `INSUFFICIENT_BALANCE`.
        code: String,
        /// Wallet-provided detail.
        message: String,
    },
    /// The wallet relay could not be reached or the request was malformed.
    #[error("wallet unreachable: {0}")]
    Unreachable(String),
}

/// A settled outgoing payment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paid {
    /// Hex preimage returned by the wallet (may be empty for some backends).
    pub preimage: String,
    /// Routing fee in msat, when the wallet reports it.
    pub fee_msat: Option<u64>,
}

/// A fresh incoming invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoice {
    /// BOLT11 string.
    pub bolt11: String,
    /// Hex payment hash, when the wallet reports it.
    pub payment_hash: Option<String>,
}

/// State of a payment or invoice looked up by hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupState {
    /// Still in flight / unpaid.
    Pending,
    /// Settled; preimage when the wallet returns it.
    Settled {
        /// Hex preimage.
        preimage: Option<String>,
        /// Amount in msat.
        amount_msat: u64,
        /// Fee in msat.
        fee_msat: u64,
        /// `true` for a payment we sent, `false` for one we received.
        outgoing: bool,
    },
    /// Failed (payments) or expired (invoices).
    Failed,
    /// The wallet does not know this hash.
    NotFound,
}

/// The operations the MCP tools need.
#[allow(async_fn_in_trait)]
pub trait Wallet: Send + Sync {
    /// Pay a BOLT11 invoice. `amount_msat` is only for amountless invoices.
    async fn pay_invoice(
        &self,
        bolt11: &str,
        amount_msat: Option<u64>,
    ) -> Result<Paid, WalletError>;
    /// Create an invoice to receive `amount_msat`.
    async fn make_invoice(
        &self,
        amount_msat: u64,
        description: Option<String>,
        expiry_secs: Option<u64>,
    ) -> Result<Invoice, WalletError>;
    /// Look a payment or invoice up by its hex payment hash.
    async fn lookup(&self, payment_hash: &str) -> Result<LookupState, WalletError>;
    /// Spendable balance in msat.
    async fn balance_msat(&self) -> Result<u64, WalletError>;
}

/// Nostr Wallet Connect client.
pub struct NwcWallet {
    client: NWC,
}

impl NwcWallet {
    /// Connect lazily to the wallet named by a `nostr+walletconnect://` URI.
    /// The URI carries a spending secret; it is never logged or returned.
    pub fn new(uri: &str, timeout: Duration) -> Result<Self, String> {
        let uri = NostrWalletConnectURI::parse(uri.trim())
            .map_err(|_| "NWC_URI is not a valid nostr+walletconnect:// URI".to_string())?;
        let opts = NostrWalletConnectOptions::new().timeout(timeout);
        Ok(Self {
            client: NWC::with_opts(uri, opts),
        })
    }
}

fn map_err(err: NwcError) -> WalletError {
    match err {
        NwcError::Timeout | NwcError::PrematureExit => WalletError::Timeout,
        NwcError::NIP47(Nip47Error::ErrorCode(e)) => WalletError::Rejected {
            code: error_code_name(&e.code).to_string(),
            message: e.message,
        },
        other => WalletError::Unreachable(other.to_string()),
    }
}

fn error_code_name(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::RateLimited => "RATE_LIMITED",
        ErrorCode::NotImplemented => "NOT_IMPLEMENTED",
        ErrorCode::InsufficientBalance => "INSUFFICIENT_BALANCE",
        ErrorCode::PaymentFailed => "PAYMENT_FAILED",
        ErrorCode::NotFound => "NOT_FOUND",
        ErrorCode::QuotaExceeded => "QUOTA_EXCEEDED",
        ErrorCode::Restricted => "RESTRICTED",
        ErrorCode::Unauthorized => "UNAUTHORIZED",
        ErrorCode::Internal => "INTERNAL",
        ErrorCode::Other => "OTHER",
    }
}

impl Wallet for NwcWallet {
    async fn pay_invoice(
        &self,
        bolt11: &str,
        amount_msat: Option<u64>,
    ) -> Result<Paid, WalletError> {
        let mut req = PayInvoiceRequest::new(bolt11);
        req.amount = amount_msat;
        let resp = self.client.pay_invoice(req).await.map_err(map_err)?;
        Ok(Paid {
            preimage: resp.preimage,
            fee_msat: resp.fees_paid,
        })
    }

    async fn make_invoice(
        &self,
        amount_msat: u64,
        description: Option<String>,
        expiry_secs: Option<u64>,
    ) -> Result<Invoice, WalletError> {
        let resp = self
            .client
            .make_invoice(MakeInvoiceRequest {
                amount: amount_msat,
                description,
                description_hash: None,
                expiry: expiry_secs,
            })
            .await
            .map_err(map_err)?;
        Ok(Invoice {
            bolt11: resp.invoice,
            payment_hash: resp.payment_hash,
        })
    }

    async fn lookup(&self, payment_hash: &str) -> Result<LookupState, WalletError> {
        let resp = match self
            .client
            .lookup_invoice(LookupInvoiceRequest {
                payment_hash: Some(payment_hash.to_string()),
                invoice: None,
            })
            .await
        {
            Ok(resp) => resp,
            Err(NwcError::NIP47(Nip47Error::ErrorCode(e))) if e.code == ErrorCode::NotFound => {
                return Ok(LookupState::NotFound)
            }
            Err(e) => return Err(map_err(e)),
        };
        let settled =
            resp.settled_at.is_some() || matches!(resp.state, Some(TransactionState::Settled));
        Ok(match resp.state {
            Some(TransactionState::Failed) | Some(TransactionState::Expired) => LookupState::Failed,
            _ if settled => LookupState::Settled {
                preimage: resp.preimage.filter(|p| !p.is_empty()),
                amount_msat: resp.amount,
                fee_msat: resp.fees_paid,
                outgoing: matches!(
                    resp.transaction_type,
                    Some(nostr::nips::nip47::TransactionType::Outgoing)
                ),
            },
            _ => LookupState::Pending,
        })
    }

    async fn balance_msat(&self) -> Result<u64, WalletError> {
        self.client.get_balance().await.map_err(map_err)
    }
}

/// Scriptable in-memory wallet for tests.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Records calls and replays canned answers.
    #[derive(Default)]
    pub struct FakeWallet {
        /// Answer for the next `pay_invoice`.
        pub pay: Mutex<Option<Result<Paid, WalletError>>>,
        /// Answer for `lookup`.
        pub lookup: Mutex<Option<Result<LookupState, WalletError>>>,
        /// `(bolt11, amount_msat)` for every pay call.
        pub pay_calls: Mutex<Vec<(String, Option<u64>)>>,
    }

    impl Wallet for FakeWallet {
        async fn pay_invoice(
            &self,
            bolt11: &str,
            amount_msat: Option<u64>,
        ) -> Result<Paid, WalletError> {
            self.pay_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((bolt11.to_string(), amount_msat));
            self.pay
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .unwrap_or(Err(WalletError::Unreachable("no answer scripted".into())))
        }

        async fn make_invoice(
            &self,
            _amount_msat: u64,
            _description: Option<String>,
            _expiry_secs: Option<u64>,
        ) -> Result<Invoice, WalletError> {
            Ok(Invoice {
                bolt11: "lnbc1fake".into(),
                payment_hash: Some("ab".repeat(32)),
            })
        }

        async fn lookup(&self, _payment_hash: &str) -> Result<LookupState, WalletError> {
            self.lookup
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .unwrap_or(Ok(LookupState::NotFound))
        }

        async fn balance_msat(&self) -> Result<u64, WalletError> {
            Ok(1_000_000)
        }
    }
}
