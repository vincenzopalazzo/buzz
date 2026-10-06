//! End to end over a real NWC relay: buzz-lightning-mcp's NWC wallet pays an
//! invoice minted by the dev-only mock wallet (no real money moves).

use std::time::Duration;

use buzz_lightning_mcp::desk::{Desk, Limits};
use buzz_lightning_mcp::wallet::NwcWallet;
use buzz_mock_wallet::{MockWallet, MockWalletConfig, Script};
use sha2::{Digest, Sha256};

const LIMITS: Limits = Limits {
    max_payment_msat: 100_000,
    budget_msat: 200_000,
};

async fn start() -> MockWallet {
    MockWallet::start(MockWalletConfig {
        balance_msat: 1_000_000,
        port: Some(0),
        receive_only: false,
        script: Script::default(),
    })
    .await
    .expect("mock wallet")
}

fn receipt_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with('⚡')).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn pays_over_nwc_and_returns_a_verifiable_sonar_receipt() {
    let mock = start().await;
    let invoice = mock.mint_payee(21_000, "coffee").expect("invoice");
    let desk = Desk::new(
        NwcWallet::new(mock.uri(), Duration::from_secs(10)).expect("uri"),
        LIMITS,
    );

    let reply = desk.pay_invoice(&invoice.bolt11, None).await;
    assert!(!reply.is_error, "{}", reply.text);
    let lines = receipt_lines(&reply.text);
    let id = reply.json["receipt_id"].as_str().expect("id");
    assert_eq!(lines[0], format!("⚡PAY|1|{id}|21"));
    let preimage = lines[1]
        .strip_prefix(&format!("⚡PAYDONE|2|{id}|"))
        .expect("settled line carries the preimage");
    let hash = reply.json["payment_hash"].as_str().expect("hash");
    assert_eq!(
        hex::encode(Sha256::digest(hex::decode(preimage).expect("hex"))),
        hash,
        "the receipt's preimage proves the payment"
    );
    assert_eq!(mock.balance_msat(), 1_000_000 - 21_000);

    // Asking again never pays again and yields the same receipt.
    let again = desk.pay_invoice(&invoice.bolt11, None).await;
    assert_eq!(again.json["already_paid"], true);
    assert_eq!(receipt_lines(&again.text), lines);
    assert_eq!(mock.balance_msat(), 1_000_000 - 21_000);
    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn wallet_error_sends_nothing_and_releases_budget() {
    let mock = start().await;
    mock.set_script(Script {
        fail_next_pay_code: Some("PAYMENT_FAILED".into()),
        ..Default::default()
    });
    let invoice = mock.mint_payee(50_000, "fails").expect("invoice");
    let desk = Desk::new(
        NwcWallet::new(mock.uri(), Duration::from_secs(10)).expect("uri"),
        LIMITS,
    );
    let reply = desk.pay_invoice(&invoice.bolt11, None).await;
    assert!(reply.is_error);
    assert!(reply.text.contains("PAYMENT_FAILED"), "{}", reply.text);
    assert!(receipt_lines(&reply.text).is_empty());
    assert_eq!(mock.balance_msat(), 1_000_000);

    // The failed attempt does not consume budget: a 150k + 50k pair still fits 200k.
    let ok = desk
        .pay_invoice(
            &mock.mint_payee(100_000, "ok").expect("invoice").bolt11,
            None,
        )
        .await;
    assert!(!ok.is_error, "{}", ok.text);
    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn silent_wallet_is_an_unknown_outcome_not_a_failure() {
    let mock = start().await;
    mock.set_script(Script {
        swallow_requests: true,
        ..Default::default()
    });
    let invoice = mock.mint_payee(21_000, "slow").expect("invoice");
    let desk = Desk::new(
        NwcWallet::new(mock.uri(), Duration::from_secs(2)).expect("uri"),
        LIMITS,
    );
    let reply = desk.pay_invoice(&invoice.bolt11, None).await;
    assert_eq!(reply.json["status"], "unknown", "{}", reply.text);
    assert!(reply.text.contains("Do not pay again"));
    assert!(receipt_lines(&reply.text).is_empty());
    let retry = desk.pay_invoice(&invoice.bolt11, None).await;
    assert!(retry.is_error && retry.text.contains("do not pay again"));
    mock.shutdown();
}
