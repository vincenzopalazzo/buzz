NIP-LP
======

Lightning Payment Requests and Receipts
---------------------------------------

`draft` `optional`

**Depends on**: NIP-01, NIP-29 (`h` scoping). Interacts with BOLT11, BOLT12, LUD-16 and BIP-353 as opaque pay targets. Does not depend on NIP-47, NIP-57 or any wallet protocol.

## Abstract

This NIP defines two channel-scoped events: a **payment request** (`kind:40009`) that asks for `N` millisatoshis against one or more Lightning pay targets, and a **payment receipt** (`kind:40010`) in which a payer reports the outcome. The relay stores and fans them out like any message. **Buzz never holds or moves funds.** Whoever pays uses a wallet they already have: an MCP wallet server driven by an agent, `lightning-cli`, a Nostr Wallet Connect client, a mobile wallet opened from a QR. The events only carry the ask and the answer, and every client derives the same card state from them without a wallet of its own.

## Motivation

Chat is where humans and agents already negotiate work. The missing primitive is not a wallet inside the chat app; it is a shared, verifiable way to say "pay me this" and "I paid" that any wallet can fulfil. Earlier attempts coupled the request/receipt format to a specific wallet transport (NIP-47) and to custody UI, which made the change too large to land. This NIP is the smallest slice: a wire format plus rendering rules. Wallets plug in from outside.

The design follows the receipt discipline Sonar uses for its `⚡PAY` / `⚡PAYDONE` chat lines (see [Relationship to other formats](#relationship-to-other-formats)): a receipt is posted only after the payer's wallet reports settlement, carries the preimage when the wallet returns one, and is treated by readers as a claim that can be upgraded to a proof.

## Non-goals

- No custody, no key material, no NWC strings, no balances anywhere in Buzz events or storage.
- No relay-side payment logic. The relay validates shape only.
- No invoice or offer decoding by the relay or by the SDK. Targets are opaque strings.
- No escrow, refunds or disputes.

## Terminology

MUST, MUST NOT, SHOULD, MAY as in RFC 2119.

- **payee**: the pubkey that will receive funds, named by the request's `p` tag.
- **payer**: the author of a receipt.
- **target**: one way to pay the request: a BOLT11 invoice, a BOLT12 offer, a LUD-16 Lightning Address or a BIP-353 human-readable address.
- **bound receipt**: a `paid` receipt whose `payment_hash` equals the request's own `payment_hash`.
- **verified**: a bound receipt whose `preimage` hashes to that `payment_hash`.

## Payment request (`kind:40009`)

A regular, `h`-scoped event. Standard signing fields omitted:

```json
{
  "kind": 40009,
  "content": "⚡ Payment request: 500 sats — lunch",
  "tags": [
    ["h", "9b353519-f4fe-4757-aef4-bec6cc0ae54c"],
    ["p", "<payee-pubkey>"],
    ["amount", "500000"],
    ["bolt11", "lnbc5u1p..."],
    ["payment_hash", "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"],
    ["lud16", "alice@example.com"],
    ["memo", "lunch"],
    ["expiry", "1700000000"]
  ]
}
```

| Tag | Required | Meaning |
| --- | --- | --- |
| `h` | yes | Channel or DM UUID (NIP-29). |
| `p` | yes | Payee pubkey, 64 lowercase hex. Usually the author. |
| `amount` | yes | Millisatoshis as an ASCII decimal string, strictly positive. No floats, no units. |
| `bolt11` | one of | BOLT11 invoice minted by the payee for exactly `amount`. |
| `bolt12` | one of | BOLT12 offer (`lno1…`). The payer fetches an invoice for `amount`. |
| `lud16` | one of | LUD-16 Lightning Address (`user@domain`). The payer resolves a fresh invoice. |
| `bip353` | one of | BIP-353 human-readable address (`user@domain`, optional `₿` prefix), resolved over DNS to an offer. |
| `payment_hash` | no | 64 lowercase hex. SHOULD be present whenever `bolt11` is; it lets clients bind receipts to this request without decoding the invoice. |
| `memo` | no | Short description, at most 512 bytes, no control characters. |
| `expiry` | no | Unix seconds after which the request is no longer payable. |

At least one target tag MUST be present. A request MAY carry several so that payers pick what their wallet supports; order is the payee's preference. Target values MUST NOT be empty or contain whitespace.

`content` SHOULD be a plain-text fallback such as `⚡ Payment request: 500 sats — lunch`. Clients that do not render cards, search indexes and push previews show the content; card-aware clients read the tags and ignore it.

**`expiry`, not `expiration`.** The tag name deliberately differs from NIP-40. A relay that implements NIP-40 auto-delete would otherwise sweep pay cards out of history. When a `bolt11` is embedded, the event `expiry` MUST NOT exceed the invoice's own expiry.

**Public channels.** A `bolt11` posted in an open channel is payable by whoever pays first; the payee then holds one payment for possibly several willing payers. For 1:1 payment use a DM, or publish `bolt12` / `lud16` / `bip353`, which mint a fresh invoice per payer.

## Payment receipt (`kind:40010`)

A regular, `h`-scoped event posted by the payer **after** its wallet reports an outcome, never before.

```json
{
  "kind": 40010,
  "content": "⚡ Paid 500 sats",
  "tags": [
    ["h", "9b353519-f4fe-4757-aef4-bec6cc0ae54c"],
    ["e", "<request-event-id>"],
    ["p", "<payee-pubkey>"],
    ["status", "paid"],
    ["amount", "500000"],
    ["payment_hash", "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"],
    ["preimage", "0000000000000000000000000000000000000000000000000000000000000000"],
    ["fee", "12"]
  ]
}
```

| Tag | Required | Meaning |
| --- | --- | --- |
| `h` | yes | Same channel as the request. |
| `e` | yes | Request event id. **Bare**: exactly `["e", "<id>"]`, no relay hint, no NIP-10 marker. |
| `status` | no | `paid` (default when absent) or `failed`. |
| `amount` | yes | Millisatoshis actually sent (or attempted). |
| `payment_hash` | for `paid` | 64 lowercase hex. |
| `preimage` | no | 64 lowercase hex. Present when the payer's wallet returned one. |
| `fee` | no | Routing fee in millisatoshis. |
| `reason` | no | Short failure reason, at most 512 bytes, for `failed`. |
| `p` | no | Payee pubkey, so the payee's client or agent is notified. |

The `e` tag MUST be bare. A marked `e` tag would route the receipt through NIP-10 threading and inflate the request's `reply_count` / `descendant_count`; relays MUST reject marked `e` tags on this kind.

A receipt MAY omit `preimage`. Some settlements never produce one at the payer (for example an ecash mint that settles two of its own users internally). Such a receipt stays a claim.

## Relay behaviour

Relays treat both kinds exactly like a `kind:9` message for authorization: `messages:write` scope, channel membership, moderation, archive checks, write quotas. Relays MUST validate the tag shape above and reject malformed events, and MUST NOT interpret, decode or verify invoices, offers, addresses or preimages. Relays MUST NOT store any wallet state.

## Client behaviour

### Deriving the card state

Clients derive one state per request from the request and every receipt whose bare `e` tag names it, in any arrival order:

1. If any receipt has `status` `paid` → **Paid**. It is **Verified** iff some such receipt is bound (`payment_hash` equals the request's `payment_hash`) and `SHA256(preimage) == payment_hash`.
2. Else if `expiry` is present and `expiry <= now` → **Expired**.
3. Else if the latest receipt has `status` `failed` → **Failed** (show `reason`).
4. Else → **Pending**.

A request without `payment_hash` can be Paid but never Verified: the preimage may be genuine, but nothing ties it to this request. `bolt12`, `lud16` and `bip353` targets mint per-payer invoices, so their requests are confirmable only by the payee's own wallet.

Reference implementations: `crates/buzz-core/src/payment.rs` (`payment_state`) and `desktop/src/features/messages/lib/payment.ts` (`derivePaymentState`).

### Rendering

A card-aware client SHOULD render a request as a card rather than as text:

- The amount in sats (`amount / 1000`, fractional msat shown only when non-zero), the memo, and the state as a badge: Pending, Expired, Failed, Paid, Verified. "Paid" and "Verified" MUST be visually distinct; "Paid" MUST NOT imply proof.
- Each target with its type (Invoice, Offer, Lightning address, BIP-353 address), truncated, with a copy action. A QR code for `bolt11` / `bolt12` when the client can render one.
- A **Pay** action that hands the preferred target (`bolt11`, then `bolt12`, then an address) to the user's wallet, for example by opening `lightning:<target>`. The client MUST NOT pay on its own.
- The receipts, each with payer, amount and whether its preimage verified.
- Expiry as relative time while pending; the Pay action hidden once Paid or Expired.

A client that does not render cards MUST show the event's `content` as a plain message. Receipts are overlays on the request: they SHOULD NOT render as their own timeline rows, and SHOULD NOT count toward unread totals.

### Trust

**A receipt is the payer's claim.** The payee's source of truth is the payee's own wallet (an incoming-payment notification or a lookup by `payment_hash`). A client acting as payee MUST confirm there before treating anything as settled; the Verified state proves that *someone* knew the preimage of the request's hash, which is strong evidence for a `bolt11` the payee minted, but it is not a wallet balance.

## Agents

Agents reach Buzz through the `buzz` CLI, so the whole flow is four commands and a wallet the agent already has:

```bash
# payee agent: mint an invoice with its wallet, then post the ask
buzz pay request --channel $CH --sats 500 --bolt11 $BOLT11 --payment-hash $HASH --memo "lunch" --expires-in 3600

# payer agent: read what is open, pay with its own wallet, report
buzz pay list --channel $CH --state pending
# <wallet tool: pay $BOLT11 → {payment_hash, preimage}>
buzz pay receipt --channel $CH --request $REQ --sats 500 --payment-hash $HASH --preimage $PREIMAGE

# anyone: audit
buzz pay show --request $REQ
buzz pay verify --preimage $PREIMAGE --payment-hash $HASH
```

The wallet is deliberately outside this NIP. A minimal wallet tool contract that fits is described in `docs/agent-payments.md`. Harnesses SHOULD wake a payee agent on receipts that `p`-tag it (Buzz's ACP harness includes `kind:40010` in its default mention kinds) and SHOULD require an owner instruction or a standing budget before an agent pays anything.

## Relationship to other formats

| Sonar `⚡PAY` chat line | This NIP |
| --- | --- |
| `⚡PAY\|1\|<uuid>\|<sats>` sent after settlement | `kind:40010` with `status=paid`; the request it answers is the `kind:40009` named by `e` |
| `⚡PAYDONE\|2\|<uuid>\|<preimage_hex>` | `preimage` tag on the receipt; `payment_hash` makes the check self-contained |
| Receiver verifies `SHA256(preimage) == payment_hash` | Verified state, identical check |
| Unknown versions render as plain text | Non-card clients render `content` |
| No request line | `kind:40009` adds the ask, the targets, the expiry and the binding hash |

NIP-57 zaps (`kind:9734` / `9735`) require an LNURL server that signs zap receipts and are tied to a note, not to a request. This NIP keeps the receipt with the payer and works for any target type, including BOLT12 and BIP-353, which zaps do not cover.

## Security considerations

- Anyone with write access can post a `paid` receipt for any request. Clients MUST render unbound or unverified receipts as claims and payees MUST check their wallet.
- A `bolt11` in an open channel can be paid by anyone; see [Payment request](#payment-request-kind40009).
- `memo` and `reason` are untrusted text, limited to 512 bytes with no control characters; clients render them as plain text, never as markup.
- Targets are opaque and can be long (a BOLT12 offer with blinded paths can exceed 1 KiB). Relays cap targets at 4 KiB and content at 4 KiB.
- Nothing in this NIP identifies a wallet, node or balance. Payers who want privacy should prefer per-payer targets.

## Registry

This NIP registers:

- `kind:40009`: Lightning payment request
- `kind:40010`: Lightning payment receipt
- tags `amount`, `bolt11`, `bolt12`, `lud16`, `bip353`, `payment_hash`, `preimage`, `memo`, `expiry`, `status`, `fee`, `reason` with the meanings above
- NIP-11 `supported_extensions`: contains `"nip-lp"` when the relay accepts these kinds (not yet advertised by Buzz)

## Acknowledgements

The tag layout, the `expiry`/`expiration` distinction and the bare-`e` rule come from Marco Pesani's NWC wallet work ([block/buzz#2635](https://github.com/block/buzz/pull/2635)). The settle-then-receipt discipline and the optional preimage follow Sonar's chat payment receipts.
