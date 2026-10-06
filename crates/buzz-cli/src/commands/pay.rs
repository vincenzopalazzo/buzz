//! `buzz pay` — NIP-LP Lightning payment requests and receipts.
//!
//! Buzz carries the request and the result; it never moves money. An agent
//! (or a human) pays with whatever wallet it already has — a wallet MCP
//! server, `lightning-cli`, an NWC connection — then reports the outcome
//! here. Clients render the request as a card and derive its state from the
//! receipts, so every reader (desktop, mobile, this CLI) agrees without a
//! wallet of its own. See `docs/nips/NIP-LP.md`.

use std::collections::HashMap;

use buzz_core::kind::{KIND_PAYMENT_RECEIPT, KIND_PAYMENT_REQUEST};
use buzz_core::payment::{
    payment_state, verify_preimage, PaymentReceipt, PaymentRequest, PaymentState, ReceiptStatus,
};
use buzz_sdk::{format_sats, PaymentReceiptParams, PaymentRequestParams};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_event_id, parse_uuid, validate_hex64};

/// Resolve `--sats` / `--msat` into millisatoshis; exactly one must be set.
fn resolve_amount_msat(sats: Option<u64>, msat: Option<u64>) -> Result<u64, CliError> {
    match (sats, msat) {
        (Some(s), None) => s
            .checked_mul(1000)
            .ok_or_else(|| CliError::Usage("--sats is too large".into())),
        (None, Some(m)) => Ok(m),
        (Some(_), Some(_)) => Err(CliError::Usage(
            "pass exactly one of --sats or --msat, not both".into(),
        )),
        (None, None) => Err(CliError::Usage(
            "an amount is required: --sats <n> or --msat <n>".into(),
        )),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `buzz pay request` — post a payment request card into a channel or DM.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_request(
    client: &BuzzClient,
    channel: &str,
    sats: Option<u64>,
    msat: Option<u64>,
    bolt11: Option<&str>,
    bolt12: Option<&str>,
    lud16: Option<&str>,
    bip353: Option<&str>,
    payment_hash: Option<&str>,
    memo: Option<&str>,
    expires_in: Option<u64>,
    payee: Option<&str>,
    content: Option<&str>,
) -> Result<(), CliError> {
    let channel_id = parse_uuid(channel)?;
    let amount_msat = resolve_amount_msat(sats, msat)?;
    let payee_pubkey = match payee {
        Some(p) => {
            validate_hex64(p)?;
            p.to_ascii_lowercase()
        }
        None => client.keys().public_key().to_hex(),
    };
    if let Some(h) = payment_hash {
        validate_hex64(h)?;
    }
    let expiry = expires_in.map(|secs| now_secs().saturating_add(secs));
    let params = PaymentRequestParams {
        amount_msat,
        payee_pubkey: &payee_pubkey,
        bolt11,
        bolt12,
        lud16,
        bip353,
        payment_hash,
        memo,
        expiry,
    };
    let builder = buzz_sdk::build_payment_request(channel_id, &params, content.unwrap_or(""))
        .map_err(|e| CliError::Usage(format!("build_payment_request failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// `buzz pay receipt` — report the outcome of paying a request.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_receipt(
    client: &BuzzClient,
    channel: &str,
    request: &str,
    sats: Option<u64>,
    msat: Option<u64>,
    payment_hash: Option<&str>,
    preimage: Option<&str>,
    fee_msat: Option<u64>,
    failed: bool,
    reason: Option<&str>,
    payee: Option<&str>,
    content: Option<&str>,
) -> Result<(), CliError> {
    let channel_id = parse_uuid(channel)?;
    let request_id = parse_event_id(request)?;
    let amount_msat = resolve_amount_msat(sats, msat)?;
    if let Some(h) = payment_hash {
        validate_hex64(h)?;
    }
    if let Some(p) = preimage {
        validate_hex64(p)?;
    }
    if let Some(p) = payee {
        validate_hex64(p)?;
    }
    let status = if failed {
        ReceiptStatus::Failed
    } else {
        ReceiptStatus::Paid
    };
    if status == ReceiptStatus::Paid && payment_hash.is_none() {
        return Err(CliError::Usage(
            "a paid receipt needs --payment-hash (pass --failed to report a failure)".into(),
        ));
    }
    let params = PaymentReceiptParams {
        status,
        amount_msat,
        payment_hash,
        preimage,
        fee_msat,
        reason,
        payee_pubkey: payee,
    };
    let builder =
        buzz_sdk::build_payment_receipt(channel_id, request_id, &params, content.unwrap_or(""))
            .map_err(|e| CliError::Usage(format!("build_payment_receipt failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// `buzz pay verify` — offline preimage check, no relay round-trip.
pub fn cmd_verify(preimage: &str, payment_hash: &str) -> Result<(), CliError> {
    validate_hex64(preimage)?;
    validate_hex64(payment_hash)?;
    let ok = verify_preimage(
        &preimage.to_ascii_lowercase(),
        &payment_hash.to_ascii_lowercase(),
    );
    println!(
        "{}",
        serde_json::json!({ "verified": ok, "payment_hash": payment_hash.to_ascii_lowercase() })
    );
    if ok {
        Ok(())
    } else {
        Err(CliError::Usage(
            "preimage does not hash to payment_hash".into(),
        ))
    }
}

fn state_label(state: PaymentState) -> &'static str {
    match state {
        PaymentState::Pending => "pending",
        PaymentState::Expired => "expired",
        PaymentState::Failed => "failed",
        PaymentState::Paid { verified: true } => "verified",
        PaymentState::Paid { verified: false } => "paid",
    }
}

fn event_tags(ev: &serde_json::Value) -> Vec<Vec<String>> {
    ev.get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_array())
                .map(|t| {
                    t.iter()
                        .map(|s| s.as_str().unwrap_or("").to_string())
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn str_field<'a>(ev: &'a serde_json::Value, name: &str) -> &'a str {
    ev.get(name).and_then(|v| v.as_str()).unwrap_or("")
}

/// Join one request with its receipts into the JSON shape every `pay`
/// read command prints.
fn render_request(
    request_ev: &serde_json::Value,
    receipts: &[serde_json::Value],
    now: u64,
) -> Option<serde_json::Value> {
    let request_id = str_field(request_ev, "id").to_string();
    let request = PaymentRequest::from_tags(&event_tags(request_ev)).ok()?;
    let parsed: Vec<(PaymentReceipt, &serde_json::Value)> = receipts
        .iter()
        .filter_map(|ev| {
            PaymentReceipt::from_tags(&event_tags(ev))
                .ok()
                .map(|r| (r, ev))
        })
        .filter(|(r, _)| r.request_id == request_id)
        .collect();
    let state = payment_state(
        &request,
        Some(&request_id),
        parsed.iter().map(|(r, _)| r),
        now,
    );
    let targets: Vec<serde_json::Value> = request
        .targets
        .iter()
        .map(|t| serde_json::json!({ "type": t.tag_name(), "value": t.value() }))
        .collect();
    let receipts_json: Vec<serde_json::Value> = parsed
        .iter()
        .map(|(r, ev)| {
            serde_json::json!({
                "id": str_field(ev, "id"),
                "payer": str_field(ev, "pubkey"),
                "created_at": ev.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "status": r.status.as_tag_value(),
                "amount_msat": r.amount.as_msat(),
                "payment_hash": r.payment_hash,
                "preimage": r.preimage,
                "verified": r.verified()
                    && request.payment_hash.is_some()
                    && request.payment_hash == r.payment_hash,
                "fee_msat": r.fee.map(|f| f.as_msat()),
                "reason": r.reason,
            })
        })
        .collect();
    Some(serde_json::json!({
        "id": request_id,
        "channel": request.channel_id,
        "payee": request.payee_pubkey,
        "author": str_field(request_ev, "pubkey"),
        "created_at": request_ev.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
        "amount_msat": request.amount.as_msat(),
        "amount": format_sats(request.amount.as_msat()),
        "memo": request.memo,
        "payment_hash": request.payment_hash,
        "expiry": request.expiry,
        "targets": targets,
        "state": state_label(state),
        "content": str_field(request_ev, "content"),
        "receipts": receipts_json,
    }))
}

async fn query_events(
    client: &BuzzClient,
    filter: serde_json::Value,
) -> Result<Vec<serde_json::Value>, CliError> {
    let raw = client.query(&filter).await?;
    serde_json::from_str::<Vec<serde_json::Value>>(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))
}

/// `buzz pay list` — requests in a channel, newest first, each with its state.
pub async fn cmd_list(
    client: &BuzzClient,
    channel: &str,
    limit: Option<u32>,
    state_filter: Option<&str>,
) -> Result<(), CliError> {
    let channel_id = parse_uuid(channel)?;
    let limit = limit.unwrap_or(50).clamp(1, 200);
    if let Some(s) = state_filter {
        if !matches!(s, "pending" | "expired" | "failed" | "paid" | "verified") {
            return Err(CliError::Usage(
                "--state must be one of pending, expired, failed, paid, verified".into(),
            ));
        }
    }
    let requests = query_events(
        client,
        serde_json::json!({
            "kinds": [KIND_PAYMENT_REQUEST],
            "#h": [channel_id.to_string()],
            "limit": limit,
        }),
    )
    .await?;
    let ids: Vec<&str> = requests.iter().map(|ev| str_field(ev, "id")).collect();
    let receipts = if ids.is_empty() {
        Vec::new()
    } else {
        query_events(
            client,
            serde_json::json!({
                "kinds": [KIND_PAYMENT_RECEIPT],
                "#h": [channel_id.to_string()],
                "#e": ids,
                "limit": 1000,
            }),
        )
        .await?
    };
    let mut by_request: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for ev in receipts {
        for t in event_tags(&ev) {
            if t.len() == 2 && t[0] == "e" {
                by_request.entry(t[1].clone()).or_default().push(ev.clone());
            }
        }
    }
    let now = now_secs();
    let empty = Vec::new();
    let mut rows: Vec<serde_json::Value> = requests
        .iter()
        .filter_map(|ev| {
            let id = str_field(ev, "id");
            render_request(ev, by_request.get(id).unwrap_or(&empty), now)
        })
        .filter(|row| {
            state_filter.is_none_or(|want| row.get("state").and_then(|v| v.as_str()) == Some(want))
        })
        .collect();
    rows.sort_by_key(|row| {
        std::cmp::Reverse(row.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0))
    });
    println!("{}", serde_json::Value::Array(rows));
    Ok(())
}

/// `buzz pay show` — one request with every receipt and the derived state.
pub async fn cmd_show(client: &BuzzClient, request: &str) -> Result<(), CliError> {
    let request_id = parse_event_id(request)?;
    let mut requests = query_events(
        client,
        serde_json::json!({
            "kinds": [KIND_PAYMENT_REQUEST],
            "ids": [request_id.to_hex()],
            "limit": 1,
        }),
    )
    .await?;
    let Some(request_ev) = requests.pop() else {
        return Err(CliError::Other(format!(
            "payment request {} not found",
            request_id.to_hex()
        )));
    };
    let receipts = query_events(
        client,
        serde_json::json!({
            "kinds": [KIND_PAYMENT_RECEIPT],
            "#e": [request_id.to_hex()],
            "limit": 200,
        }),
    )
    .await?;
    let row = render_request(&request_ev, &receipts, now_secs())
        .ok_or_else(|| CliError::Other("stored payment request has an invalid tag shape".into()))?;
    println!("{row}");
    Ok(())
}

pub async fn dispatch(cmd: crate::PayCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::PayCmd;
    match cmd {
        PayCmd::Request {
            channel,
            sats,
            msat,
            bolt11,
            bolt12,
            lud16,
            bip353,
            payment_hash,
            memo,
            expires_in,
            payee,
            content,
        } => {
            cmd_request(
                client,
                &channel,
                sats,
                msat,
                bolt11.as_deref(),
                bolt12.as_deref(),
                lud16.as_deref(),
                bip353.as_deref(),
                payment_hash.as_deref(),
                memo.as_deref(),
                expires_in,
                payee.as_deref(),
                content.as_deref(),
            )
            .await
        }
        PayCmd::Receipt {
            channel,
            request,
            sats,
            msat,
            payment_hash,
            preimage,
            fee_msat,
            failed,
            reason,
            payee,
            content,
        } => {
            cmd_receipt(
                client,
                &channel,
                &request,
                sats,
                msat,
                payment_hash.as_deref(),
                preimage.as_deref(),
                fee_msat,
                failed,
                reason.as_deref(),
                payee.as_deref(),
                content.as_deref(),
            )
            .await
        }
        PayCmd::List {
            channel,
            limit,
            state,
        } => cmd_list(client, &channel, limit, state.as_deref()).await,
        PayCmd::Show { request } => cmd_show(client, &request).await,
        PayCmd::Verify {
            preimage,
            payment_hash,
        } => cmd_verify(&preimage, &payment_hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
    const PAYEE: &str = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    const REQUEST_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const PREIMAGE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const HASH: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

    #[test]
    fn amount_resolution() {
        assert_eq!(resolve_amount_msat(Some(21), None).unwrap(), 21_000);
        assert_eq!(resolve_amount_msat(None, Some(5)).unwrap(), 5);
        assert!(resolve_amount_msat(Some(1), Some(1)).is_err());
        assert!(resolve_amount_msat(None, None).is_err());
        assert!(resolve_amount_msat(Some(u64::MAX), None).is_err());
    }

    #[test]
    fn render_joins_receipts_and_derives_state() {
        let request = serde_json::json!({
            "id": REQUEST_ID,
            "pubkey": PAYEE,
            "created_at": 1_700_000_000u64,
            "kind": KIND_PAYMENT_REQUEST,
            "content": "⚡ Payment request: 5 sats",
            "tags": [
                ["h", CHANNEL], ["p", PAYEE], ["amount", "5000"],
                ["bolt11", "lnbc50n1pexample"], ["payment_hash", HASH], ["memo", "coffee"],
            ],
        });
        let paid = serde_json::json!({
            "id": "f".repeat(64),
            "pubkey": "e".repeat(64),
            "created_at": 1_700_000_100u64,
            "kind": KIND_PAYMENT_RECEIPT,
            "tags": [
                ["h", CHANNEL], ["e", REQUEST_ID], ["status", "paid"], ["amount", "5000"],
                ["payment_hash", HASH], ["preimage", PREIMAGE],
            ],
        });
        let other = serde_json::json!({
            "id": "d".repeat(64),
            "pubkey": "e".repeat(64),
            "created_at": 1_700_000_100u64,
            "kind": KIND_PAYMENT_RECEIPT,
            "tags": [
                ["h", CHANNEL], ["e", "c".repeat(64)], ["status", "paid"], ["amount", "1"],
                ["payment_hash", HASH],
            ],
        });
        let row = render_request(&request, &[other, paid], 1_700_000_200).unwrap();
        assert_eq!(row["state"], "verified");
        assert_eq!(row["amount"], "5 sats");
        assert_eq!(row["memo"], "coffee");
        assert_eq!(row["targets"][0]["type"], "bolt11");
        assert_eq!(row["receipts"].as_array().unwrap().len(), 1);
        assert_eq!(row["receipts"][0]["verified"], true);

        let row = render_request(&request, &[], 1_700_000_200).unwrap();
        assert_eq!(row["state"], "pending");
    }

    #[test]
    fn verify_reports_mismatch_as_usage_error() {
        assert!(cmd_verify(PREIMAGE, HASH).is_ok());
        assert!(matches!(
            cmd_verify(PREIMAGE, REQUEST_ID),
            Err(CliError::Usage(_))
        ));
    }
}
