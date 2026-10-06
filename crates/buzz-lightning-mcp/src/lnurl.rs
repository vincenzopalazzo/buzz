//! LUD-16 Lightning Address → BOLT11 invoice (LUD-06 payRequest).
//!
//! Only resolves; paying stays in the wallet. Every response is size-capped
//! and time-limited, and the returned invoice must ask for exactly the amount
//! requested, so a hostile LNURL server cannot inflate the payment.

use std::time::Duration;

use serde::Deserialize;

use crate::bolt11;

/// Largest LNURL response body accepted.
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PayRequest {
    callback: String,
    min_sendable: u64,
    max_sendable: u64,
    tag: Option<String>,
    #[serde(default)]
    comment_allowed: Option<u64>,
}

#[derive(Deserialize)]
struct CallbackResponse {
    pr: Option<String>,
    status: Option<String>,
    reason: Option<String>,
}

/// Split `user@domain` into the well-known LNURL-pay URL.
pub fn well_known_url(address: &str) -> Result<String, String> {
    let address = address.trim().trim_start_matches('₿');
    let (user, domain) = address
        .split_once('@')
        .ok_or_else(|| format!("not a Lightning Address: {address}"))?;
    let user_ok = !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
    let domain_ok = !domain.is_empty()
        && domain.contains('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.'));
    if !user_ok || !domain_ok {
        return Err(format!("not a Lightning Address: {address}"));
    }
    Ok(format!(
        "https://{}/.well-known/lnurlp/{}",
        domain.to_ascii_lowercase(),
        user.to_ascii_lowercase()
    ))
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    http: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let mut resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("LNURL request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("LNURL server answered HTTP {}", resp.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("LNURL read failed: {e}"))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err("LNURL response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|e| format!("LNURL response is not valid JSON: {e}"))
}

/// Resolve `address` to a BOLT11 invoice for exactly `amount_msat`.
pub async fn resolve_invoice(
    address: &str,
    amount_msat: u64,
    comment: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    let pay: PayRequest = get_json(&http, &well_known_url(address)?).await?;
    if pay.tag.as_deref().is_some_and(|t| t != "payRequest") {
        return Err("Lightning Address did not return an LNURL payRequest".into());
    }
    if amount_msat < pay.min_sendable || amount_msat > pay.max_sendable {
        return Err(format!(
            "amount out of range for this address: {}..={} sats",
            pay.min_sendable.div_ceil(1000),
            pay.max_sendable / 1000
        ));
    }
    let mut callback = reqwest::Url::parse(&pay.callback)
        .map_err(|_| "LNURL callback is not a URL".to_string())?;
    if callback.scheme() != "https" {
        return Err("LNURL callback must use https".into());
    }
    callback
        .query_pairs_mut()
        .append_pair("amount", &amount_msat.to_string());
    if let (Some(c), Some(max)) = (comment, pay.comment_allowed) {
        if max > 0 && !c.is_empty() {
            let c: String = c.chars().take(max as usize).collect();
            callback.query_pairs_mut().append_pair("comment", &c);
        }
    }
    let resp: CallbackResponse = get_json(&http, callback.as_str()).await?;
    if resp.status.as_deref() == Some("ERROR") {
        return Err(format!(
            "LNURL server refused: {}",
            resp.reason.unwrap_or_default()
        ));
    }
    let pr = resp
        .pr
        .ok_or_else(|| "LNURL server returned no invoice".to_string())?;
    let info = bolt11::inspect(&pr)?;
    if info.amount_msat != Some(amount_msat) {
        return Err("LNURL invoice amount does not match the requested amount".into());
    }
    Ok(info.invoice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_well_known_url() {
        assert_eq!(
            well_known_url("Alice@Example.com").unwrap(),
            "https://example.com/.well-known/lnurlp/alice"
        );
        assert_eq!(
            well_known_url("₿bob@pay.example.org").unwrap(),
            "https://pay.example.org/.well-known/lnurlp/bob"
        );
        for bad in [
            "alice",
            "@example.com",
            "alice@",
            "a b@example.com",
            "alice@localhost",
            "alice@ex/ample.com",
        ] {
            assert!(well_known_url(bad).is_err(), "{bad}");
        }
    }
}
