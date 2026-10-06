# Agent payments: bring your own wallet MCP, Buzz shows the bubble

`draft`

Buzz shows Lightning payments in a conversation and holds no wallet. People
already run a wallet MCP server for their agent, for example
[lexe-mcp](https://github.com/vincenzopalazzo/lexe-mcp) for a Lexe node, or an
NWC or Core Lightning server. The agent pays with that tool. Buzz then tells
the agent to end its reply with two receipt lines in
[Sonar](https://github.com/hedwig-corp/bitchat-to-sonar)'s chat receipt wire
format, and the Buzz UI renders those lines as a payment bubble.

Buzz adds no event kind, relay logic, CLI command or wallet code for this. It
adds one prompt section for its agents, and parsing in the desktop and mobile
timelines.

## The pieces

| Piece | Where | Role |
| --- | --- | --- |
| Wallet MCP | the user's own, e.g. lexe-mcp as a goose extension | Pays, enforces its own limits, reports settlement. |
| Agent | goose (or any agent) under `buzz-acp` | Pays with the wallet tool, then posts its reply with `buzz messages send`. |
| Receipt contract | `crates/buzz-acp/src/base_prompt.md`, section *Payment Receipts* | Tells the agent how to turn a settled payment into the two lines. |
| UI | Buzz Desktop and mobile | Turns the lines into a gold bubble. |

With lexe-mcp, a settled `lexe.pay` returns the following, and the agent turns
it into `⚡PAY|1|<index>|<amount>` and `⚡PAYDONE|2|<index>`:

- `settled: true`
- `amount` in sats
- `index`, in the form `<created_at>-<payment id>`
- for BOLT12 offers, a payer proof with a `proof_url` on lnproof.space

The proof link goes in the reply's prose. A pending or unknown result gets no
lines; the agent checks it later with `lexe.check_payment`.

## Wire format

Byte-for-byte Sonar (`docs/SONAR-PAYMENTS.md`, decoder `SonarPay.kt`):

```text
⚡PAY|1|<id>|<sats>                payment receipt: a gold bubble
⚡PAYDONE|2|<id>                   settled, no preimage available
⚡PAYDONE|2|<id>|<preimage_hex>    settled, with the 32-byte Lightning preimage
```

Decoding follows Sonar's rules:

- Fields are split on `|`.
- `⚡PAY` needs version `1` and a positive whole number of sats.
- `⚡PAYDONE` takes version `2` with an optional 64-hex preimage, or version
  `1` with no preimage.
- Anything else stays plain text, including unknown versions and `⚡PAYCLAIM`.

**Lines inside a longer message.** Sonar sends each line as a whole message.
An agent's reply wraps them in prose, so Buzz also finds them inside a
message. Each must sit on its own line, with no leading space; trailing
whitespace is ignored. The receipt lines are removed from the displayed text,
and one bubble is drawn per `⚡PAY` line. Text mentioning `⚡PAY|1|…`
mid-sentence, or on an indented line, stays text.

## How Buzz renders it

- **Status.** A `⚡PAY` line shows the amount in sats. The status reads
  "Payment pending" until settled, then "Paid". It describes what the message
  author did, and the row header names the author. Sonar says "Received" for
  the other person's receipt because its chats are one-to-one; in a Buzz
  channel the viewer is rarely the payee.
- **Hidden rows.** A message containing only `⚡PAYDONE` lines is a hidden
  control row. It never appears in the timeline, search, the home feed or
  activity, and it raises no unread count or notification: live, in the
  desktop's native catch-up when the app opens, on mobile, or in an iOS push.
- **Previews.** Anywhere a message is shown as one line (search, inbox,
  notifications, push, reminders, link previews, the reply banner), receipt
  lines read "⚡ Paid 2,100 sats" or "⚡ 2,100 sats payment".
- **Settlement.** A `⚡PAYDONE` settles the `⚡PAY` with the same id. It can
  arrive in the same message, a later one, or an earlier one; the timeline
  folds the whole conversation.
- **Signer binding.** Sonar chats are one-to-one. A Buzz channel has many
  writers, so a `⚡PAYDONE` only settles a `⚡PAY` signed by the same key.
  Nobody can mark someone else's receipt paid.
- **Proof.** A preimage shows as a "proof" control that copies it. Buzz cannot
  verify it, because the payment hash lives in the payee's wallet. The payee
  checks it there. For BOLT12, the payer proof link the wallet returns, such
  as lexe-mcp's lnproof.space URL, is the stronger proof; the agent puts it in
  the prose.
- **Edits and deletions.** An edit that adds or removes lines changes the
  bubble. A deleted `⚡PAYDONE` stops settling.

Implementation:

- desktop: `desktop/src/features/messages/lib/sonarPay.ts` (decoder and
  `sonarPayPreviewText`), `formatTimelineMessages.ts`, `ui/SonarPayBubble.tsx`,
  `features/channels/useLiveChannelUpdates.ts`, and
  `src/shared/lib/remarkBip353.ts`, which keeps a `₿alice@example.com`
  BIP-353 name from becoming a `mailto:` link
- desktop native: `desktop/src-tauri/src/sonar_pay.rs`, used by the unread
  catch-up and the home feed's mentions
- mobile: `mobile/lib/shared/sonar_pay/`, `features/channels/timeline_message.dart`,
  the channel and thread message views, notifications, activity and search
- iOS push: `mobile/ios/BuzzPushKit/Sources/BuzzPushKit/SonarPayPreview.swift`

## Payment rails

The receipt does not care how the payment was made. A BOLT11 invoice, a
BOLT12 offer, and a BIP-353 name (`₿alice@example.com`, a DNS name that
resolves to a BOLT12 offer) all settle to the same two lines. Whether a rail
works depends on the wallet MCP: lexe-mcp pays BOLT11 and BOLT12 today, and
refuses `user@domain` names it cannot resolve. Buzz only needs to render what
the agent writes. Long invoices and offers wrap inside the message, and a
BIP-353 name written with its `₿` prefix stays plain text on desktop and
mobile.

## Why this shape

The earlier attempt, [block/buzz#2635](https://github.com/block/buzz/pull/2635),
put a whole NWC wallet inside Buzz:

- custody and keyring storage
- NWC and LNURL adapters
- a desktop wallet UI
- new event kinds and relay ingest

It touched 179 files with 27.6k lines. Its author closed it the day it opened,
and the fork has been idle since July 2026.

Sonar shows the smaller split: payments settle in a wallet, and the chat
carries only a receipt. Here the wallet sits behind whatever MCP server the
user already runs. Buzz's part shrinks to a receipt contract and rendering, so
any wallet works without Buzz shipping or reviewing wallet code.

## Known gaps

- A bubble is the payer's claim. The payee should confirm in their own wallet
  before acting on it.
- The agent model writes the lines from the wallet result, so a model can
  get them wrong. A malformed line then shows as plain text, never as a
  bubble. A wallet MCP that returns the two lines ready to paste would remove
  that step.
- Previews and the native filters look at a message's own content. An edit
  that turns a message into a `⚡PAYDONE`-only row is hidden in the timeline,
  but its original text can still show in search or a stale notification.
