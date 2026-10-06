import assert from "node:assert/strict";
import test from "node:test";

import {
  collectSonarPaySettlements,
  decodeSonarPayLine,
  describeSonarPay,
  isSonarPayControlLine,
  parseSonarPayContent,
  resolveSonarPayMessage,
} from "./sonarPay.ts";

// Single-receipt view of a message, for the ordering tests below.
const viewOf = (content, signer, settlements) =>
  resolveSonarPayMessage(content, signer, settlements)?.receipts[0];

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
  assert.deepEqual(viewOf("⚡PAY|1|p1|21", ALICE, settlements), {
    id: "p1",
    sats: 21,
    settled: true,
    preimage: PREIMAGE,
  });
  assert.deepEqual(viewOf("⚡PAY|1|p2|500", ALICE.toUpperCase(), settlements), {
    id: "p2",
    sats: 500,
    settled: false,
    preimage: undefined,
  });
  assert.equal(viewOf("hello", ALICE, settlements), undefined);
});

test("a DONE with a preimage wins over a bare DONE", () => {
  const settlements = collectSonarPaySettlements([
    { pubkey: ALICE, content: `⚡PAYDONE|2|p1|${PREIMAGE}` },
    { pubkey: ALICE, content: "⚡PAYDONE|2|p1" },
  ]);
  assert.equal(viewOf("⚡PAY|1|p1|1", ALICE, settlements)?.preimage, PREIMAGE);
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

test("finds receipt lines inside an agent's sentence", () => {
  const content = [
    "Done, I paid the invoice for the coffee.",
    "⚡PAY|1|p9|21",
    `⚡PAYDONE|2|p9|${PREIMAGE}  `,
    "",
    "Anything else?",
  ].join("\r\n");
  const parsed = parseSonarPayContent(content);
  assert.deepEqual(parsed?.pays, [{ id: "p9", sats: 21 }]);
  assert.deepEqual(parsed?.dones, [{ id: "p9", preimage: PREIMAGE }]);
  assert.equal(
    parsed?.text,
    "Done, I paid the invoice for the coffee.\r\n\r\nAnything else?",
  );

  // Same message settles its own bubble.
  const view = resolveSonarPayMessage(
    content,
    ALICE,
    collectSonarPaySettlements([{ pubkey: ALICE, content }]),
  );
  assert.equal(view?.receipts.length, 1);
  assert.equal(view?.receipts[0].settled, true);
  assert.equal(view?.receipts[0].preimage, PREIMAGE);
  assert.match(view?.text ?? "", /^Done, I paid/);
});

test("indented or inline receipt text is not a receipt", () => {
  assert.equal(parseSonarPayContent("  ⚡PAY|1|p1|21"), null);
  assert.equal(parseSonarPayContent("I sent ⚡PAY|1|p1|21 earlier"), null);
  assert.equal(parseSonarPayContent("```\n⚡PAY|1|p1|21x\n```"), null);
});

test("a message of only DONE lines is a hidden control row", () => {
  assert.equal(isSonarPayControlLine("⚡PAYDONE|2|a\n⚡PAYDONE|2|b"), true);
  assert.equal(isSonarPayControlLine("settled\n⚡PAYDONE|2|a"), false);
  // Text with only DONE lines renders as that text, without the lines.
  assert.deepEqual(
    resolveSonarPayMessage("settled\n⚡PAYDONE|2|a", ALICE, new Map()),
    { text: "settled", receipts: [] },
  );
  assert.equal(
    resolveSonarPayMessage("⚡PAYDONE|2|a", ALICE, new Map()),
    undefined,
  );
});

test("preview text summarizes receipts and skips DONE-only messages", async () => {
  const { sonarPayPreviewText } = await import("./sonarPay.ts");
  assert.equal(sonarPayPreviewText("hello"), "hello");
  assert.equal(
    sonarPayPreviewText(
      `Paid the coffee.\n⚡PAY|1|g1|2100\n⚡PAYDONE|2|g1|${PREIMAGE}`,
    ),
    "Paid the coffee. ⚡ Paid 2,100 sats",
  );
  assert.equal(sonarPayPreviewText("⚡PAY|1|g1|21"), "⚡ 21 sats payment");
  assert.equal(sonarPayPreviewText("⚡PAYDONE|2|g1"), null);
  assert.equal(sonarPayPreviewText("settled\n⚡PAYDONE|2|g1"), "settled");
});
