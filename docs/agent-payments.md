# Agent payments: Sonar receipts in Buzz

`draft`

Buzz shows Lightning payments in a conversation without holding a wallet. Whoever
paid (a human, or an agent with its own wallet tool) posts a receipt in
[Sonar](https://github.com/hedwig-corp/bitchat-to-sonar)'s chat receipt wire
format. Buzz renders it as a payment bubble. Buzz never pays, never stores keys
or balances, and adds no event kind, relay logic or schema.

## Wire format

Each line is the **entire** content of an ordinary message (`kind:9` or
`kind:40002`). It is byte-for-byte Sonar's format (`docs/SONAR-PAYMENTS.md`,
decoder `SonarPay.kt`), so Sonar and Buzz clients read the same messages.

```text
⚡PAY|1|<id>|<sats>                payment receipt: rendered as a gold bubble
⚡PAYDONE|2|<id>                   settled, no preimage available
⚡PAYDONE|2|<id>|<preimage_hex>    settled, with the 32-byte Lightning preimage
```

Decoding rules, mirrored from Sonar:

- `|` splits the fields. The line must start at the first character: a leading
  space makes it plain text.
- `⚡PAY` needs version `1` and a positive integer amount in sats.
- `⚡PAYDONE` accepts version `2` with an optional 64-hex preimage, and version
  `1` with no preimage (old peers).
- Anything else, including unknown versions and `⚡PAYCLAIM`, renders as plain
  text.

## How Buzz renders it

- A `⚡PAY` message renders as a gold bubble with the amount in sats. The
  status line reads "Sending" until settled, then "Paid" for your own receipts
  or "Received" for someone else's.
- A `⚡PAYDONE` message is a hidden control row. It never shows in the timeline
  and never raises unread or a notification. It only flips the matching bubble
  to settled.
- A `⚡PAYDONE` can arrive before its `⚡PAY`. The timeline folds the whole
  conversation, so arrival order does not matter.
- **Signer binding.** Sonar conversations are 1:1. A Buzz channel has many
  writers, so a `⚡PAYDONE` only settles a `⚡PAY` signed by the same key. Nobody
  can mark someone else's receipt paid.
- When a preimage is present, the bubble shows a "proof" control that copies
  it. Buzz cannot check it: the payment hash lives in the payee's wallet. The
  payee checks it with their wallet or `buzz pay verify`.
- A deleted `⚡PAYDONE` stops settling its bubble.

Implementation: `desktop/src/features/messages/lib/sonarPay.ts` (decoder and
fold), `formatTimelineMessages.ts` (hide and attach), `SonarPayBubble.tsx`
(bubble), `useLiveChannelUpdates.ts` (no unread or notification for
`⚡PAYDONE`).

## How an agent reports a payment

The agent pays with whatever wallet it was given: an MCP wallet server,
`lightning-cli`, an NWC client, a Cashu wallet. Then it sends the result back
with the `buzz` CLI, which every managed agent already has on `PATH`:

```bash
# after the wallet returned {preimage}
buzz pay receipt --channel $CH --sats 21 --preimage $PREIMAGE [--reply-to $EVENT]
# → posts ⚡PAY|1|<id>|21, then ⚡PAYDONE|2|<id>|<preimage>
#   {"pay_id":"<id>","receipt":{...},"done":{...}}

# a payment still in flight: post the receipt now, settle later
buzz pay receipt --channel $CH --sats 500 --pending
buzz pay done --channel $CH --id <pay_id> [--preimage $PREIMAGE]

# as the payee, check a preimage against the hash of your own invoice
buzz pay verify --preimage $PREIMAGE --payment-hash $HASH
```

The agent base prompt (`crates/buzz-acp/src/base_prompt.md`) sets two rules:

- Never pay without an explicit instruction or a standing owner budget.
- Post a receipt only after the wallet reports the payment settled.

A bubble is the payer's claim. A payee acts on it only after confirming in its
own wallet.

### Recommended wallet tool contract (outside Buzz)

Any wallet an agent can call works. A minimal MCP surface that fits:

| Tool | Input | Output |
| --- | --- | --- |
| `wallet_pay` | `{ target, amount_sat?, max_fee_sat? }`: a BOLT11 invoice, BOLT12 offer, LUD-16 or BIP-353 address | `{ status: "paid" \| "failed", payment_hash?, preimage?, amount_sat, fee_sat?, reason? }` |
| `wallet_invoice` | `{ amount_sat, description?, expiry_secs? }` | `{ bolt11, payment_hash, expiry }` |
| `wallet_lookup` | `{ payment_hash }` | `{ status: "settled" \| "pending" \| "failed" \| "not_found", preimage? }` |

`wallet_pay` should fire at most once per payment. A timeout is an unknown
outcome to resolve with `wallet_lookup`, never by paying again.

## Why this shape

The earlier attempt, [block/buzz#2635](https://github.com/block/buzz/pull/2635),
"Lightning Wallet (NWC)", put a whole wallet product into Buzz. Its author
closed it the day it opened, and its fork has been idle since July 2026. It
covered:

- custody and NWC/LNURL adapters
- a mock wallet daemon and keyring storage
- a desktop wallet UI
- new event kinds and relay ingest

| | |
| --- | --- |
| Files / lines | 179 / 27.6k |

Most of it was custody, which a chat relay does not need to own.

Sonar already showed the smaller shape: payments settle in a wallet and the
chat only carries a receipt. Buzz adopts that receipt format as is. The change
is client rendering plus a CLI helper.

## Known gaps

- Mobile shows the raw lines as text until it gets a bubble.
- The desktop's cold-start unread catch-up runs natively and still counts a
  `⚡PAYDONE` as a message. Live delivery is filtered.
- Sidebar, search and notification previews show the raw `⚡PAY` line.
