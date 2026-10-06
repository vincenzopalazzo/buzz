//! `buzz pay` — post Sonar chat payment receipts into a Buzz conversation.
//!
//! Buzz holds no wallet and never pays. Whoever paid (a human, or an agent
//! with its own wallet tool) reports the outcome with Sonar's chat receipt
//! wire format, carried as the entire content of an ordinary kind:9 message:
//!
//! ```text
//! ⚡PAY|1|<id>|<sats>              receipt, rendered as a gold bubble
//! ⚡PAYDONE|2|<id>                 settled, no preimage available
//! ⚡PAYDONE|2|<id>|<preimage_hex>  settled, with the Lightning preimage
//! ```
//!
//! Buzz clients render the `⚡PAY` line as a payment bubble and use the hidden
//! `⚡PAYDONE` line to mark it settled. Sonar clients read the same lines.
//! Spec: Sonar `docs/SONAR-PAYMENTS.md`; Buzz notes: `docs/agent-payments.md`.

use sha2::{Digest, Sha256};

use crate::client::{normalize_write_response, BuzzClient};
use crate::commands::messages::resolve_thread_ref;
use crate::error::CliError;
use crate::validate::{parse_uuid, validate_hex64};

/// Longest receipt id accepted. Sonar emits 16 hex chars.
const MAX_PAY_ID_LEN: usize = 64;

/// `⚡PAY|1|<id>|<sats>`.
pub(crate) fn pay_line(id: &str, sats: u64) -> String {
    format!("⚡PAY|1|{id}|{sats}")
}

/// `⚡PAYDONE|2|<id>` or `⚡PAYDONE|2|<id>|<preimage>`.
pub(crate) fn done_line(id: &str, preimage: Option<&str>) -> String {
    match preimage {
        Some(p) => format!("⚡PAYDONE|2|{id}|{p}"),
        None => format!("⚡PAYDONE|2|{id}"),
    }
}

/// A fresh receipt id: 16 lowercase hex chars, like Sonar's `randomPayId()`.
fn new_pay_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

/// Ids travel inside a `|`-delimited line, so keep them to a safe alphabet.
fn validate_pay_id(id: &str) -> Result<(), CliError> {
    let ok = !id.is_empty()
        && id.len() <= MAX_PAY_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CliError::Usage(format!(
            "--id must be 1-{MAX_PAY_ID_LEN} characters of [A-Za-z0-9_-]: {id}"
        )))
    }
}

fn normalize_preimage(preimage: Option<&str>) -> Result<Option<String>, CliError> {
    match preimage {
        None => Ok(None),
        Some(p) => {
            validate_hex64(p)?;
            Ok(Some(p.to_ascii_lowercase()))
        }
    }
}

/// `true` iff `SHA256(preimage) == payment_hash` (both 64-char hex).
pub(crate) fn preimage_matches(preimage: &str, payment_hash: &str) -> bool {
    let (Ok(pre), Ok(hash)) = (hex::decode(preimage), hex::decode(payment_hash)) else {
        return false;
    };
    pre.len() == 32 && hash.len() == 32 && Sha256::digest(&pre).as_slice() == hash.as_slice()
}

async fn post_line(
    client: &BuzzClient,
    channel: uuid::Uuid,
    content: &str,
    thread_ref: Option<&buzz_sdk::ThreadRef>,
) -> Result<serde_json::Value, CliError> {
    let builder = buzz_sdk::build_message(channel, content, thread_ref, &[], false, &[], &[])
        .map_err(|e| CliError::Other(format!("build_message failed: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    serde_json::from_str(&normalize_write_response(&resp))
        .map_err(|e| CliError::Other(format!("unexpected relay response: {e}")))
}

fn accepted(write: &serde_json::Value) -> bool {
    write.get("accepted").and_then(|v| v.as_bool()) == Some(true)
}

/// `buzz pay receipt` — post `⚡PAY`, then (unless `--pending`) `⚡PAYDONE`.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_receipt(
    client: &BuzzClient,
    channel: &str,
    sats: u64,
    preimage: Option<&str>,
    id: Option<&str>,
    reply_to: Option<&str>,
    pending: bool,
) -> Result<(), CliError> {
    let channel = parse_uuid(channel)?;
    if sats == 0 {
        return Err(CliError::Usage("--sats must be greater than zero".into()));
    }
    if pending && preimage.is_some() {
        return Err(CliError::Usage(
            "--preimage means the payment settled; drop --pending".into(),
        ));
    }
    let preimage = normalize_preimage(preimage)?;
    let id = match id {
        Some(id) => {
            validate_pay_id(id)?;
            id.to_string()
        }
        None => new_pay_id(),
    };
    let thread_ref = match reply_to {
        Some(r) => {
            validate_hex64(r)?;
            Some(resolve_thread_ref(client, r).await?)
        }
        None => None,
    };

    let receipt = post_line(client, channel, &pay_line(&id, sats), thread_ref.as_ref()).await?;
    if !accepted(&receipt) {
        println!(
            "{}",
            serde_json::json!({ "pay_id": id, "receipt": receipt, "done": null })
        );
        return Err(CliError::Other("relay rejected the ⚡PAY receipt".into()));
    }
    let done = if pending {
        serde_json::Value::Null
    } else {
        post_line(
            client,
            channel,
            &done_line(&id, preimage.as_deref()),
            thread_ref.as_ref(),
        )
        .await?
    };
    println!(
        "{}",
        serde_json::json!({ "pay_id": id, "receipt": receipt, "done": done })
    );
    if !done.is_null() && !accepted(&done) {
        return Err(CliError::Other(format!(
            "relay rejected the ⚡PAYDONE line; retry with: buzz pay done --channel {channel} --id {id}"
        )));
    }
    Ok(())
}

/// `buzz pay done` — settle an earlier `⚡PAY` you posted.
pub async fn cmd_done(
    client: &BuzzClient,
    channel: &str,
    id: &str,
    preimage: Option<&str>,
    reply_to: Option<&str>,
) -> Result<(), CliError> {
    let channel = parse_uuid(channel)?;
    validate_pay_id(id)?;
    let preimage = normalize_preimage(preimage)?;
    let thread_ref = match reply_to {
        Some(r) => {
            validate_hex64(r)?;
            Some(resolve_thread_ref(client, r).await?)
        }
        None => None,
    };
    let done = post_line(
        client,
        channel,
        &done_line(id, preimage.as_deref()),
        thread_ref.as_ref(),
    )
    .await?;
    println!("{}", serde_json::json!({ "pay_id": id, "done": done }));
    if accepted(&done) {
        Ok(())
    } else {
        Err(CliError::Other("relay rejected the ⚡PAYDONE line".into()))
    }
}

/// `buzz pay verify` — offline `SHA256(preimage) == payment_hash` check.
pub fn cmd_verify(preimage: &str, payment_hash: &str) -> Result<(), CliError> {
    validate_hex64(preimage)?;
    validate_hex64(payment_hash)?;
    let verified = preimage_matches(preimage, payment_hash);
    println!(
        "{}",
        serde_json::json!({
            "verified": verified,
            "payment_hash": payment_hash.to_ascii_lowercase(),
        })
    );
    if verified {
        Ok(())
    } else {
        Err(CliError::Usage(
            "preimage does not hash to payment_hash".into(),
        ))
    }
}

pub async fn dispatch(cmd: crate::PayCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::PayCmd;
    match cmd {
        PayCmd::Receipt {
            channel,
            sats,
            preimage,
            id,
            reply_to,
            pending,
        } => {
            cmd_receipt(
                client,
                &channel,
                sats,
                preimage.as_deref(),
                id.as_deref(),
                reply_to.as_deref(),
                pending,
            )
            .await
        }
        PayCmd::Done {
            channel,
            id,
            preimage,
            reply_to,
        } => {
            cmd_done(
                client,
                &channel,
                &id,
                preimage.as_deref(),
                reply_to.as_deref(),
            )
            .await
        }
        PayCmd::Verify {
            preimage,
            payment_hash,
        } => cmd_verify(&preimage, &payment_hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREIMAGE: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    // SHA256 of 32 zero bytes.
    const HASH: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

    // Byte-for-byte the strings Sonar's `PayLine.encoded()` produces.
    #[test]
    fn lines_match_sonar_encoding() {
        assert_eq!(pay_line("abc-123", 21), "⚡PAY|1|abc-123|21");
        assert_eq!(done_line("abc-123", None), "⚡PAYDONE|2|abc-123");
        assert_eq!(
            done_line("abc-123", Some(PREIMAGE)),
            format!("⚡PAYDONE|2|abc-123|{PREIMAGE}")
        );
    }

    #[test]
    fn generated_ids_are_sixteen_hex_chars() {
        let id = new_pay_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(validate_pay_id(&id).is_ok());
        assert_ne!(id, new_pay_id());
    }

    #[test]
    fn ids_cannot_break_the_wire_format() {
        for bad in ["", "a|b", "has space", "é", &"x".repeat(65)] {
            assert!(validate_pay_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn preimages_are_lowercased_and_validated() {
        assert_eq!(
            normalize_preimage(Some(&PREIMAGE.to_uppercase())).unwrap(),
            Some(PREIMAGE.to_string())
        );
        assert!(normalize_preimage(Some("abc")).is_err());
        assert_eq!(normalize_preimage(None).unwrap(), None);
    }

    #[test]
    fn verify_checks_sha256() {
        assert!(preimage_matches(PREIMAGE, HASH));
        assert!(!preimage_matches(PREIMAGE, PREIMAGE));
        assert!(cmd_verify(PREIMAGE, HASH).is_ok());
        assert!(matches!(
            cmd_verify(PREIMAGE, PREIMAGE),
            Err(CliError::Usage(_))
        ));
    }
}
