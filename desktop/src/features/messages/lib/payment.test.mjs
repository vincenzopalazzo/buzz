import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import test from "node:test";

import {
  derivePaymentState,
  formatSats,
  getPaymentReceiptTargetId,
  parsePaymentReceiptTags,
  parsePaymentRequestTags,
  verifyPaymentReceipt,
} from "./payment.ts";

const CHANNEL = "9b353519-f4fe-4757-aef4-bec6cc0ae54c";
const PAYEE = "ab".repeat(32);
const REQUEST_ID = "01".repeat(32);
const PREIMAGE = "00".repeat(32);
const HASH = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

const requestTags = () => [
  ["h", CHANNEL],
  ["p", PAYEE],
  ["amount", "500000"],
  ["bolt11", "lnbc5u1pexample"],
  ["payment_hash", HASH],
  ["memo", "lunch"],
  ["expiry", "1700000000"],
];

const receipt = (over = {}) => ({
  id: "ff".repeat(32),
  payerPubkey: "ee".repeat(32),
  payerDisplayName: "bob",
  createdAt: 1_700_000_100,
  channelId: CHANNEL,
  requestId: REQUEST_ID,
  status: "paid",
  amountMsat: 500_000,
  paymentHash: HASH,
  ...over,
});

test("parses a full payment request", () => {
  const req = parsePaymentRequestTags(requestTags());
  assert.deepEqual(req, {
    amountMsat: 500_000,
    channelId: CHANNEL,
    payeePubkey: PAYEE,
    targets: [{ type: "bolt11", value: "lnbc5u1pexample" }],
    paymentHash: HASH,
    memo: "lunch",
    expiry: 1_700_000_000,
  });
});

test("rejects requests with no target, zero amount, or bad hash", () => {
  assert.equal(
    parsePaymentRequestTags(requestTags().filter((t) => t[0] !== "bolt11")),
    null,
  );
  assert.equal(
    parsePaymentRequestTags(
      requestTags().map((t) => (t[0] === "amount" ? ["amount", "0"] : t)),
    ),
    null,
  );
  assert.equal(
    parsePaymentRequestTags(
      requestTags().map((t) =>
        t[0] === "payment_hash" ? ["payment_hash", "nope"] : t,
      ),
    ),
    null,
  );
  assert.equal(parsePaymentRequestTags(undefined), null);
});

test("parses receipts, defaulting status to paid and requiring a bare e tag", () => {
  const r = parsePaymentReceiptTags([
    ["h", CHANNEL],
    ["e", REQUEST_ID],
    ["amount", "500000"],
    ["payment_hash", HASH],
    ["preimage", PREIMAGE],
    ["fee", "12"],
  ]);
  assert.equal(r?.status, "paid");
  assert.equal(r?.feeMsat, 12);
  assert.equal(
    getPaymentReceiptTargetId(
      r && [
        ["h", CHANNEL],
        ["e", REQUEST_ID],
        ["amount", "1"],
        ["payment_hash", HASH],
      ],
    ),
    REQUEST_ID,
  );
  assert.equal(
    parsePaymentReceiptTags([
      ["h", CHANNEL],
      ["e", REQUEST_ID, "", "reply"],
      ["amount", "1"],
      ["payment_hash", HASH],
    ]),
    null,
  );
  assert.equal(
    parsePaymentReceiptTags([
      ["h", CHANNEL],
      ["e", REQUEST_ID],
      ["amount", "1"],
    ]),
    null,
    "paid receipts need a payment hash",
  );
  const failed = parsePaymentReceiptTags([
    ["h", CHANNEL],
    ["e", REQUEST_ID],
    ["amount", "1"],
    ["status", "failed"],
    ["reason", "no route"],
  ]);
  assert.equal(failed?.status, "failed");
  assert.equal(failed?.reason, "no route");
});

test("derives pending, expired, failed, paid and verified states", () => {
  const req = parsePaymentRequestTags(requestTags());
  assert.deepEqual(derivePaymentState(req, [], 1_600_000_000), {
    kind: "pending",
  });
  assert.deepEqual(derivePaymentState(req, [], 1_700_000_000), {
    kind: "expired",
  });
  const failed = receipt({
    id: "aa".repeat(32),
    status: "failed",
    reason: "no route",
  });
  assert.deepEqual(derivePaymentState(req, [failed], 1_600_000_000), {
    kind: "failed",
    reason: "no route",
  });
  assert.deepEqual(derivePaymentState(req, [failed], 1_700_000_001), {
    kind: "expired",
  });
  const claimed = receipt();
  assert.deepEqual(derivePaymentState(req, [failed, claimed], 1_800_000_000), {
    kind: "paid",
    verified: false,
  });
  assert.deepEqual(
    derivePaymentState(req, [claimed], 1_800_000_000, new Set([claimed.id])),
    { kind: "paid", verified: true },
  );
  // A verified preimage for a hash the request never published stays a claim.
  const unbound = { ...req, paymentHash: undefined };
  assert.deepEqual(
    derivePaymentState(
      unbound,
      [claimed],
      1_800_000_000,
      new Set([claimed.id]),
    ),
    { kind: "paid", verified: false },
  );
});

test("verifies a preimage with Web Crypto", async () => {
  assert.equal(
    await verifyPaymentReceipt(
      { preimage: PREIMAGE, paymentHash: HASH },
      webcrypto.subtle,
    ),
    true,
  );
  assert.equal(
    await verifyPaymentReceipt(
      { preimage: PREIMAGE, paymentHash: REQUEST_ID },
      webcrypto.subtle,
    ),
    false,
  );
  assert.equal(
    await verifyPaymentReceipt({ paymentHash: HASH }, webcrypto.subtle),
    false,
  );
});

test("formats sats at the UI boundary", () => {
  assert.equal(formatSats(1_000), "1 sats");
  assert.equal(formatSats(21_500), "21.5 sats");
  assert.equal(formatSats(21_001), "21.001 sats");
});
