/**
 * NIP-LP Lightning payment requests (kind 40009) and receipts (kind 40010).
 *
 * Pure tag parsing and state derivation, mirroring `buzz_core::payment` in
 * Rust so every client reaches the same card state from the same events.
 * No wallet here: Buzz carries the request and the result, nothing else.
 * See `docs/nips/NIP-LP.md`.
 */

export type PaymentTargetType = "bolt11" | "bolt12" | "lud16" | "bip353";

export type PaymentTarget = {
  type: PaymentTargetType;
  value: string;
};

export type ParsedPaymentRequest = {
  amountMsat: number;
  channelId: string;
  payeePubkey: string;
  targets: PaymentTarget[];
  paymentHash?: string;
  memo?: string;
  /** Unix seconds. */
  expiry?: number;
};

export type PaymentReceiptStatus = "paid" | "failed";

export type ParsedPaymentReceipt = {
  channelId: string;
  requestId: string;
  status: PaymentReceiptStatus;
  amountMsat: number;
  paymentHash?: string;
  preimage?: string;
  feeMsat?: number;
  reason?: string;
};

/** A receipt joined to its request row by the timeline formatter. */
export type PaymentReceiptSummary = ParsedPaymentReceipt & {
  id: string;
  payerPubkey: string;
  payerDisplayName: string;
  createdAt: number;
};

export type PaymentState =
  | { kind: "pending" }
  | { kind: "expired" }
  | { kind: "failed"; reason?: string }
  | { kind: "paid"; verified: boolean };

const HEX64 = /^[0-9a-f]{64}$/;
const DIGITS = /^[0-9]+$/;
const TARGET_TAGS: ReadonlySet<string> = new Set([
  "bolt11",
  "bolt12",
  "lud16",
  "bip353",
]);

function firstTag(tags: string[][], name: string): string | undefined {
  return tags.find((t) => t[0] === name)?.[1];
}

function parseMsat(raw: string | undefined): number | undefined {
  if (!raw || !DIGITS.test(raw)) return undefined;
  const n = Number(raw);
  return Number.isSafeInteger(n) ? n : undefined;
}

function hex64(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const lower = raw.toLowerCase();
  return HEX64.test(lower) ? lower : undefined;
}

/** Parse kind-40009 tags; `null` when the shape is invalid. */
export function parsePaymentRequestTags(
  tags: string[][] | undefined,
): ParsedPaymentRequest | null {
  if (!tags) return null;
  const amountMsat = parseMsat(firstTag(tags, "amount"));
  const channelId = firstTag(tags, "h");
  const payeePubkey = hex64(firstTag(tags, "p"));
  if (!amountMsat || amountMsat <= 0 || !channelId || !payeePubkey) {
    return null;
  }
  const targets: PaymentTarget[] = [];
  for (const tag of tags) {
    const [name, value] = tag;
    if (!name || !TARGET_TAGS.has(name)) continue;
    const trimmed = value?.trim();
    if (!trimmed || /\s/.test(trimmed)) return null;
    targets.push({ type: name as PaymentTargetType, value: trimmed });
  }
  if (targets.length === 0) return null;
  const rawHash = firstTag(tags, "payment_hash");
  const paymentHash = hex64(rawHash);
  if (rawHash && !paymentHash) return null;
  const rawExpiry = firstTag(tags, "expiry");
  const expiry = rawExpiry === undefined ? undefined : parseMsat(rawExpiry);
  if (rawExpiry !== undefined && expiry === undefined) return null;
  const memo = firstTag(tags, "memo")?.trim() || undefined;
  return {
    amountMsat,
    channelId,
    payeePubkey,
    targets,
    paymentHash,
    memo,
    expiry,
  };
}

/** Parse kind-40010 tags; `null` when the shape is invalid. */
export function parsePaymentReceiptTags(
  tags: string[][] | undefined,
): ParsedPaymentReceipt | null {
  if (!tags) return null;
  const channelId = firstTag(tags, "h");
  const eTag = tags.find((t) => t[0] === "e");
  // Bare `["e", id]` only — a marker would make this a NIP-10 reply.
  const requestId = eTag && eTag.length === 2 ? hex64(eTag[1]) : undefined;
  const amountMsat = parseMsat(firstTag(tags, "amount"));
  if (!channelId || !requestId || amountMsat === undefined) return null;
  const rawStatus = firstTag(tags, "status");
  let status: PaymentReceiptStatus;
  if (rawStatus === undefined || rawStatus === "paid") status = "paid";
  else if (rawStatus === "failed") status = "failed";
  else return null;
  const rawHash = firstTag(tags, "payment_hash");
  const paymentHash = hex64(rawHash);
  if (rawHash && !paymentHash) return null;
  if (status === "paid" && !paymentHash) return null;
  const rawPreimage = firstTag(tags, "preimage");
  const preimage = hex64(rawPreimage);
  if (rawPreimage && !preimage) return null;
  const rawFee = firstTag(tags, "fee");
  const feeMsat = rawFee === undefined ? undefined : parseMsat(rawFee);
  const reason = firstTag(tags, "reason")?.trim() || undefined;
  return {
    channelId,
    requestId,
    status,
    amountMsat,
    paymentHash,
    preimage,
    feeMsat,
    reason,
  };
}

/** The request id a receipt points at, when its `e` tag is bare and valid. */
export function getPaymentReceiptTargetId(
  tags: string[][] | undefined,
): string | undefined {
  return parsePaymentReceiptTags(tags)?.requestId;
}

/**
 * Fold a request and its receipts into a card state. `verifiedReceiptIds`
 * holds the receipts whose preimage was checked against the request's own
 * `payment_hash` (see `verifyPaymentReceipt`); without that check a paid
 * receipt is a claim, never a proof.
 */
export function derivePaymentState(
  request: ParsedPaymentRequest,
  receipts: readonly PaymentReceiptSummary[],
  nowSeconds: number,
  verifiedReceiptIds: ReadonlySet<string> = new Set(),
): PaymentState {
  let paid = false;
  let verified = false;
  let lastFailure: PaymentReceiptSummary | undefined;
  for (const receipt of receipts) {
    if (receipt.status === "paid") {
      paid = true;
      if (
        verifiedReceiptIds.has(receipt.id) &&
        request.paymentHash !== undefined &&
        receipt.paymentHash === request.paymentHash
      ) {
        verified = true;
      }
    } else if (!lastFailure || receipt.createdAt > lastFailure.createdAt) {
      lastFailure = receipt;
    }
  }
  if (paid) return { kind: "paid", verified };
  if (request.expiry !== undefined && request.expiry <= nowSeconds) {
    return { kind: "expired" };
  }
  if (lastFailure) return { kind: "failed", reason: lastFailure.reason };
  return { kind: "pending" };
}

/** `500 sats`, `21.5 sats` — sats only at the UI boundary. */
export function formatSats(amountMsat: number): string {
  const sats = Math.floor(amountMsat / 1000);
  const rem = amountMsat % 1000;
  if (rem === 0) return `${sats.toLocaleString()} sats`;
  const frac = String(rem).padStart(3, "0").replace(/0+$/, "");
  return `${sats.toLocaleString()}.${frac} sats`;
}

function hexToBytes(hex: string): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(new ArrayBuffer(hex.length / 2));
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function bytesToHex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * `SHA256(preimage) == payment_hash`, computed with Web Crypto. Resolves
 * `false` for receipts without a preimage or when the digest is unavailable.
 */
export async function verifyPaymentReceipt(
  receipt: Pick<ParsedPaymentReceipt, "preimage" | "paymentHash">,
  subtle: SubtleCrypto | undefined = globalThis.crypto?.subtle,
): Promise<boolean> {
  if (!receipt.preimage || !receipt.paymentHash || !subtle) return false;
  try {
    const digest = await subtle.digest("SHA-256", hexToBytes(receipt.preimage));
    return bytesToHex(digest) === receipt.paymentHash;
  } catch {
    return false;
  }
}

/** `lightning:` URI a wallet can open for the first payable target. */
export function paymentTargetUri(target: PaymentTarget): string {
  return `lightning:${target.value}`;
}

export function paymentTargetLabel(type: PaymentTargetType): string {
  switch (type) {
    case "bolt11":
      return "Invoice";
    case "bolt12":
      return "Offer";
    case "lud16":
      return "Lightning address";
    case "bip353":
      return "BIP-353 address";
  }
}
