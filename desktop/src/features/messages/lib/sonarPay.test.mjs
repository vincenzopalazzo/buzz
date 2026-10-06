import assert from "node:assert/strict";
import test from "node:test";

import {
  collectSonarPaySettlements,
  decodeSonarPayLine,
  describeSonarPay,
  isSonarPayControlLine,
  resolveSonarPayView,
} from "./sonarPay.ts";

const PREIMAGE = "00".repeat(32);
const ALICE = "aa".repeat(32);
const BOB = "bb".repeat(32);

// Vectors mirror Sonar's SonarPay.kt / TranscriptDisplayPolicyTest.kt.
test("decodes the three Sonar lines", () => {
  assert.deepEqual(decodeSonarPayLine("⚡PAY|1|abc-123|21"), {
    type: "pay",
    id: "abc-123",
    sats: 21,
  });
  assert.deepEqual(decodeSonarPayLine("⚡PAYDONE|2|abc-123"), {
    type: "done",
    id: "abc-123",
  });
  assert.deepEqual(decodeSonarPayLine(`⚡PAYDONE|2|abc-123|${PREIMAGE}`), {
    type: "done",
    id: "abc-123",
    preimage: PREIMAGE,
  });
  assert.deepEqual(decodeSonarPayLine("⚡PAYDONE|1|abc-123"), {
    type: "done",
    id: "abc-123",
  });
});

test("rejects everything Sonar renders as plain text", () => {
  for (const content of [
    "⚡PAY|2|abc|21", // unknown version
    "⚡PAY|1|abc|0", // non-positive amount
    "⚡PAY|1|abc|-5",
    "⚡PAY|1|abc|1.5",
    "⚡PAY|1|abc", // missing amount
    "⚡PAYDONE|3|abc", // unknown version
    "⚡PAYDONE|1|abc|extra", // v1 has no preimage
    "⚡PAYDONE|2|abc|nothex", // malformed preimage
    `⚡PAYDONE|2|abc|${PREIMAGE}|x`,
    " ⚡PAYDONE|1|abc-123", // leading space: not a control line
    "⚡PAYCLAIM|1|abc|21", // not part of the protocol
    "paid you 21 sats",
    "",
  ]) {
    assert.equal(decodeSonarPayLine(content), null, content);
  }
});

test("only DONE lines are hidden control rows", () => {
  assert.equal(isSonarPayControlLine("⚡PAYDONE|2|abc"), true);
  assert.equal(isSonarPayControlLine("⚡PAY|1|abc|21"), false);
  assert.equal(isSonarPayControlLine("hello"), false);
});

test("DONE settles only a PAY from the same signer, in any order", () => {
  const events = [
    { pubkey: ALICE, content: `⚡PAYDONE|2|p1|${PREIMAGE}` }, // races ahead
    { pubkey: ALICE, content: "⚡PAY|1|p1|21" },
    { pubkey: BOB, content: "⚡PAYDONE|2|p2" }, // forged for Alice's p2
    { pubkey: ALICE, content: "⚡PAY|1|p2|500" },
  ];
  const settlements = collectSonarPaySettlements(events);
  assert.deepEqual(resolveSonarPayView("⚡PAY|1|p1|21", ALICE, settlements), {
    id: "p1",
    sats: 21,
    settled: true,
    preimage: PREIMAGE,
  });
  assert.deepEqual(
    resolveSonarPayView("⚡PAY|1|p2|500", ALICE.toUpperCase(), settlements),
    { id: "p2", sats: 500, settled: false, preimage: undefined },
  );
  assert.equal(resolveSonarPayView("hello", ALICE, settlements), undefined);
});

test("a DONE with a preimage wins over a bare DONE", () => {
  const settlements = collectSonarPaySettlements([
    { pubkey: ALICE, content: `⚡PAYDONE|2|p1|${PREIMAGE}` },
    { pubkey: ALICE, content: "⚡PAYDONE|2|p1" },
  ]);
  assert.equal(
    resolveSonarPayView("⚡PAY|1|p1|1", ALICE, settlements)?.preimage,
    PREIMAGE,
  );
});

test("describes the bubble for previews", () => {
  assert.equal(
    describeSonarPay({ id: "x", sats: 2100, settled: true }),
    "Paid 2,100 sats",
  );
  assert.equal(
    describeSonarPay({ id: "x", sats: 21, settled: false }),
    "Sending 21 sats",
  );
});
