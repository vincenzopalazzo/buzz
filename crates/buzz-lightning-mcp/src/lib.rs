//! `buzz-lightning-mcp`: an MCP server that pays Lightning invoices through
//! the user's own wallet (Nostr Wallet Connect) and returns a Sonar chat
//! receipt that Buzz and Sonar render as a payment bubble.
//!
//! Add it to any MCP-capable agent (goose, Buzz's own agent, Claude Code).
//! After a settled payment the tool result contains two lines:
//!
//! ```text
//! ⚡PAY|1|<id>|<sats>
//! ⚡PAYDONE|2|<id>|<preimage>
//! ```
//!
//! The agent copies them into its chat reply, and the Buzz UI turns them into
//! the bubble. Buzz itself holds no wallet.
//!
//! Configuration (environment):
//! - `NWC_URI` (or `NWC_CONNECTION_STRING`): `nostr+walletconnect://…`
//!   connection string with a spending budget set in your wallet.
//! - `BUZZ_LIGHTNING_MAX_PAYMENT_SATS` (default 10 000): per-payment cap.
//! - `BUZZ_LIGHTNING_BUDGET_SATS` (default 50 000): total this process may
//!   spend, settled or in flight.
//! - `BUZZ_LIGHTNING_TIMEOUT_SECS` (default 60): wallet and LNURL timeout.

#![forbid(unsafe_code)]

pub mod bolt11;
pub mod desk;
pub mod lnurl;
pub mod receipt;
pub mod wallet;

use std::sync::Arc;
use std::time::Duration;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

use desk::{Desk, Limits, Reply};
use wallet::NwcWallet;

const DEFAULT_MAX_PAYMENT_SATS: u64 = 10_000;
const DEFAULT_BUDGET_SATS: u64 = 50_000;
const DEFAULT_TIMEOUT_SECS: u64 = 60;

const INSTRUCTIONS: &str =
    "Lightning payments through the user's own wallet (Nostr Wallet Connect). \
Only pay when the user explicitly asked you to, or within a budget they set. \
Each invoice is paid at most once. If a payment's outcome is unknown, never pay again: \
call check_payment. When a payment settles, the result includes a two-line chat receipt \
(lines starting with ⚡PAY and ⚡PAYDONE). Put those lines in your chat reply exactly as \
written, each on its own line and outside any code block, so Buzz and Sonar show a \
payment bubble. Never write receipt lines yourself.";

/// Runtime configuration read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    nwc_uri: Option<String>,
    limits: Limits,
    timeout: Duration,
}

fn env_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v
            .trim()
            .parse()
            .map_err(|_| format!("{name} must be a whole number")),
        _ => Ok(default),
    }
}

impl Config {
    /// Read `NWC_URI` and the limits from the environment.
    pub fn from_env() -> Result<Self, String> {
        let nwc_uri = std::env::var("NWC_URI")
            .or_else(|_| std::env::var("NWC_CONNECTION_STRING"))
            .ok()
            .filter(|v| !v.trim().is_empty());
        let max = env_u64("BUZZ_LIGHTNING_MAX_PAYMENT_SATS", DEFAULT_MAX_PAYMENT_SATS)?;
        let budget = env_u64("BUZZ_LIGHTNING_BUDGET_SATS", DEFAULT_BUDGET_SATS)?;
        let timeout = env_u64("BUZZ_LIGHTNING_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS)?.clamp(5, 600);
        Ok(Self {
            nwc_uri,
            limits: Limits {
                max_payment_msat: max.saturating_mul(1000),
                budget_msat: budget.saturating_mul(1000),
            },
            timeout: Duration::from_secs(timeout),
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PayInvoiceParams {
    /// BOLT11 invoice to pay (`lnbc…`, a `lightning:` prefix is fine).
    pub invoice: String,
    /// Amount in sats. Required only when the invoice has no amount; must match otherwise.
    #[serde(default)]
    pub amount_sats: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PayAddressParams {
    /// Lightning Address (LUD-16), e.g. `alice@getalby.com`.
    pub address: String,
    /// Amount in sats.
    pub amount_sats: u64,
    /// Optional note for the recipient, when their address accepts comments.
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckPaymentParams {
    /// Hex payment hash from an earlier pay or invoice result.
    pub payment_hash: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInvoiceParams {
    /// Amount to receive, in sats.
    pub amount_sats: u64,
    /// Description shown to the payer.
    #[serde(default)]
    pub description: Option<String>,
    /// Seconds until the invoice expires (wallet default when omitted).
    #[serde(default)]
    pub expiry_secs: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

fn to_result(reply: Reply) -> CallToolResult {
    let content = vec![
        ContentBlock::text(reply.text),
        ContentBlock::text(reply.json.to_string()),
    ];
    if reply.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

#[derive(Clone)]
struct LightningMcp {
    desk: Option<Arc<Desk<NwcWallet>>>,
    setup_error: Option<String>,
    timeout: Duration,
    tool_router: ToolRouter<LightningMcp>,
}

impl LightningMcp {
    fn desk(&self) -> Result<&Desk<NwcWallet>, CallToolResult> {
        self.desk.as_deref().ok_or_else(|| {
            to_result(Reply {
                text: self.setup_error.clone().unwrap_or_else(|| {
                    "No wallet configured: set NWC_URI to a nostr+walletconnect:// \
                     connection string in this MCP server's environment."
                        .into()
                }),
                json: serde_json::json!({ "status": "error", "error": "wallet_not_configured" }),
                is_error: true,
            })
        })
    }
}

#[tool_router]
impl LightningMcp {
    fn new(config: Config) -> Self {
        let (desk, setup_error) = match config.nwc_uri.as_deref() {
            None => (None, None),
            Some(uri) => match NwcWallet::new(uri, config.timeout) {
                Ok(w) => (Some(Arc::new(Desk::new(w, config.limits))), None),
                Err(e) => (None, Some(e)),
            },
        };
        Self {
            desk,
            setup_error,
            timeout: config.timeout,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "pay_invoice",
        description = "Pay a BOLT11 Lightning invoice from the user's wallet. Only when the user asked you to pay. Returns the payment result and, once settled, a two-line chat receipt (⚡PAY / ⚡PAYDONE) to copy verbatim into your chat reply so Buzz shows a payment bubble."
    )]
    async fn pay_invoice(
        &self,
        Parameters(p): Parameters<PayInvoiceParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let desk = match self.desk() {
            Ok(d) => d,
            Err(e) => return Ok(e),
        };
        Ok(to_result(desk.pay_invoice(&p.invoice, p.amount_sats).await))
    }

    #[tool(
        name = "pay_lightning_address",
        description = "Pay a Lightning Address (user@domain, LUD-16) a given amount in sats from the user's wallet. Only when the user asked you to pay. Returns a two-line chat receipt to copy verbatim into your chat reply once settled."
    )]
    async fn pay_lightning_address(
        &self,
        Parameters(p): Parameters<PayAddressParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let desk = match self.desk() {
            Ok(d) => d,
            Err(e) => return Ok(e),
        };
        if p.amount_sats == 0 {
            return Ok(to_result(Reply {
                text: "amount_sats must be greater than zero".into(),
                json: serde_json::json!({ "status": "error" }),
                is_error: true,
            }));
        }
        let invoice = match lnurl::resolve_invoice(
            &p.address,
            p.amount_sats.saturating_mul(1000),
            p.comment.as_deref(),
            self.timeout,
        )
        .await
        {
            Ok(i) => i,
            Err(e) => {
                return Ok(to_result(Reply {
                    text: format!("Could not get an invoice from {}: {e}", p.address),
                    json: serde_json::json!({ "status": "error", "error": e }),
                    is_error: true,
                }))
            }
        };
        Ok(to_result(desk.pay_invoice(&invoice, None).await))
    }

    #[tool(
        name = "check_payment",
        description = "Look up a payment or invoice by payment hash. Use it when a payment's outcome was unknown (never pay again instead) or to see whether an invoice you created was paid. Returns the chat receipt for a settled outgoing payment."
    )]
    async fn check_payment(
        &self,
        Parameters(p): Parameters<CheckPaymentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let desk = match self.desk() {
            Ok(d) => d,
            Err(e) => return Ok(e),
        };
        Ok(to_result(desk.check_payment(&p.payment_hash).await))
    }

    #[tool(
        name = "create_invoice",
        description = "Create a BOLT11 invoice so someone can pay the user. Returns the invoice and its payment hash."
    )]
    async fn create_invoice(
        &self,
        Parameters(p): Parameters<CreateInvoiceParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let desk = match self.desk() {
            Ok(d) => d,
            Err(e) => return Ok(e),
        };
        Ok(to_result(
            desk.create_invoice(p.amount_sats, p.description, p.expiry_secs)
                .await,
        ))
    }

    #[tool(
        name = "get_balance",
        description = "Show the wallet balance and how much this session may still spend."
    )]
    async fn get_balance(
        &self,
        Parameters(_): Parameters<NoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let desk = match self.desk() {
            Ok(d) => d,
            Err(e) => return Ok(e),
        };
        Ok(to_result(desk.balance().await))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LightningMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "buzz-lightning-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS.to_string())
    }
}

/// Serve over stdio until the client disconnects.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let _ = rustls::crypto::ring::default_provider().install_default();
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .init();
            let config = Config::from_env()?;
            let service = LightningMcp::new(config).serve(stdio()).await?;
            service.waiting().await?;
            Ok(())
        })
}
