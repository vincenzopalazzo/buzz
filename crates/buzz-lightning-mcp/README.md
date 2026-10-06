# buzz-lightning-mcp

An MCP server that pays Lightning invoices from **your own wallet**, over
[Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md).
It returns a chat receipt that Buzz and [Sonar](https://github.com/hedwig-corp/bitchat-to-sonar)
render as a payment bubble.

Buzz holds no wallet. The agent pays with this server, then pastes the receipt
lines into its chat reply. The Buzz UI turns them into the bubble.

```text
you (in Buzz)  ──▶  goose  ──pay_invoice──▶  buzz-lightning-mcp  ──NWC──▶  your wallet
                      ◀──── settled + receipt lines ────
goose  ──buzz messages send──▶  "Paid it.\n⚡PAY|1|<id>|21\n⚡PAYDONE|2|<id>|<preimage>"
Buzz   ──▶  "Paid it." + gold 21-sat bubble, settled, with a copyable preimage
```

## Tools

| Tool | What it does |
| --- | --- |
| `pay_invoice` | Pays a BOLT11 invoice. `amount_sats` is only for invoices without an amount. |
| `pay_lightning_address` | Resolves a Lightning Address (`user@domain`, LUD-16) to an invoice for exactly `amount_sats`, then pays it. |
| `check_payment` | Looks a payment or invoice up by payment hash. Use it after an unknown outcome. |
| `create_invoice` | Creates an invoice so someone can pay you. |
| `get_balance` | Shows the wallet balance and what this session may still spend. |

After a settled payment the result ends with two lines:

```text
⚡PAY|1|<id>|<sats>
⚡PAYDONE|2|<id>|<preimage_hex>
```

The agent puts them in its reply, each on its own line. This is Sonar's chat
receipt wire format byte for byte, so Sonar clients read it too.

Buzz renders the `⚡PAY` line as the bubble. It hides the `⚡PAYDONE` line and
uses it to mark the bubble settled. The rest of the reply renders as normal
text.

## Safety rules

- **Spending limits.** Each payment is capped at `BUZZ_LIGHTNING_MAX_PAYMENT_SATS`.
  The process spends at most `BUZZ_LIGHTNING_BUDGET_SATS` in total, counting
  payments in flight. Both are checked before the wallet is called. Set a
  budget in your wallet's NWC connection too.
- **Each invoice is paid at most once per process.** Paying again returns the
  first receipt instead of paying twice.
- **A timeout is an unknown outcome, not a failure.** The payment may still
  settle, so its budget stays reserved and paying again is refused.
  `check_payment` resolves it and returns the receipt once settled.
- **A receipt only after settlement.** The preimage is included only when it
  hashes to the invoice's payment hash.
- **Lightning Address checks.** Lookups require HTTPS, cap responses at 64 KiB
  and time out. The returned invoice must ask for exactly the requested amount.
- The connection string can spend. It is read from the environment, and the
  server never logs it or returns it.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `NWC_URI` (or `NWC_CONNECTION_STRING`) | none | `nostr+walletconnect://…` from your wallet: Alby Hub, LNbits, Phoenixd, Core Lightning with an NWC plugin, and others. |
| `BUZZ_LIGHTNING_MAX_PAYMENT_SATS` | `10000` | Largest single payment. |
| `BUZZ_LIGHTNING_BUDGET_SATS` | `50000` | Total this process may spend. |
| `BUZZ_LIGHTNING_TIMEOUT_SECS` | `60` | Wallet and Lightning Address timeout, clamped to 5–600. |

Without `NWC_URI` the server still starts, and every tool explains how to
configure it.

## Use it with goose in Buzz

1. Build the server:

   ```bash
   cargo build --release -p buzz-lightning-mcp   # → target/release/buzz-lightning-mcp
   ```

2. Add it as a goose extension in `~/.config/goose/config.yaml`. You can also
   use `goose configure` → *Add Extension* → *Command-line Extension*:

   ```yaml
   extensions:
     lightning:
       name: lightning
       type: stdio
       enabled: true
       cmd: /path/to/buzz-lightning-mcp
       args: []
       env_keys: [NWC_URI]          # stored in goose's secret store by `goose configure`
       envs:
         BUZZ_LIGHTNING_MAX_PAYMENT_SATS: "5000"
         BUZZ_LIGHTNING_BUDGET_SATS: "20000"
       timeout: 300
   ```

   Putting `NWC_URI` directly under `envs` also works, but leaves the secret
   in the file.

3. Start your goose agent in Buzz as usual. Buzz Desktop lists the extension
   under the agent's MCP servers. Then ask it in a channel:

   > @goose pay lnbc210n1… for the coffee

   goose calls `pay_invoice` and posts its reply with `buzz messages send`.
   The reply includes the two receipt lines, and Buzz shows the bubble.

The server has no Buzz dependency. It works the same in Buzz's own agent,
Claude Code, or any MCP client.

## Try it without real money

`buzz-mock-wallet` is a local NWC wallet with an in-memory ledger. It mints
real BOLT11 invoices with genuine preimages and moves no money:

```bash
cargo run -p buzz-mock-wallet -- --balance-msat 1000000
# prints: nostr+walletconnect://…   ← use it as NWC_URI
```

The end-to-end tests drive this server through that wallet over a real NWC
relay. They cover:

- a settled payment with a verifiable preimage
- no double payment
- a wallet error that sends nothing and releases the budget
- a silent wallet that becomes an unknown outcome

Run them with:

```bash
cargo test -p buzz-lightning-mcp
```
