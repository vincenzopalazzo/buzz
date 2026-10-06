# buzz-mock-wallet

**Dev-only** local Nostr Wallet Connect (NIP-47) wallet for end-to-end testing.

This crate moves **no real money**. It binds loopback only, keeps an in-memory
msat ledger, and mints real bolt11 invoices with genuine preimage/hash pairs so
clients can exercise pay / receive / lookup against a live NWC URI.

## Usage

```bash
cargo run -p buzz-mock-wallet -- --balance-msat 1000000
# stdout: nostr+walletconnect://…   (use it as NWC_URI for buzz-lightning-mcp)
```

Flags script failures for tests: `--fail-next-pay PAYMENT_FAILED`,
`--response-delay-ms`, `--swallow-requests` (no answer, for client timeouts),
`--receive-only`.

Originally written by Marco Pesani for the Lightning wallet work in
[block/buzz#2635](https://github.com/block/buzz/pull/2635); here it backs the
end-to-end tests of `crates/buzz-lightning-mcp` and local demos.
