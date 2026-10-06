import assert from "node:assert/strict";
import test from "node:test";
import { fromMarkdown } from "mdast-util-from-markdown";
import remarkGfm from "remark-gfm";

import remarkBip353 from "./remarkBip353.ts";

// Parse with the exact GFM extensions production registers through
// `remark-gfm`, so the test sees the autolinker's real output.
function parse(markdown) {
  const data = {};
  remarkGfm.call({ data: () => data });
  const tree = fromMarkdown(markdown, {
    extensions: data.micromarkExtensions,
    mdastExtensions: data.fromMarkdownExtensions,
  });
  remarkBip353()(tree);
  return tree.children[0].children;
}

const summary = (nodes) =>
  nodes.map((n) => (n.type === "link" ? `link:${n.url}` : `text:${n.value}`));

test("a BIP-353 name stays text, not a mailto link", () => {
  assert.deepEqual(summary(parse("Paid ₿alice@example.com now.")), [
    "text:Paid ₿alice@example.com now.",
  ]);
});

test("plain email addresses are still autolinked", () => {
  assert.deepEqual(
    summary(parse("Paid ₿alice@example.com, mail bob@example.com")),
    ["text:Paid ₿alice@example.com, mail ", "link:mailto:bob@example.com"],
  );
});

test("explicit mailto links after ₿ are left alone", () => {
  assert.deepEqual(summary(parse("₿[alice](mailto:alice@example.com)")), [
    "text:₿",
    "link:mailto:alice@example.com",
  ]);
});

test("works inside formatting", () => {
  const [strong] = parse("**₿carol@pay.example.org**");
  assert.equal(strong.type, "strong");
  assert.deepEqual(summary(strong.children), ["text:₿carol@pay.example.org"]);
});
