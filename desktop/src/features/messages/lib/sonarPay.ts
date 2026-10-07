/**
 * Sonar chat payment receipts, rendered by Buzz without a wallet.
 *
 * Wire format (Sonar `docs/SONAR-PAYMENTS.md`, decoder `SonarPay.kt`): plain
 * control strings carried as the *entire* content of an ordinary message.
 *
 *   ⚡PAY|1|<id>|<sats>             receipt, shown as a gold bubble
 *   ⚡PAYDONE|2|<id>                settled, no preimage available
 *   ⚡PAYDONE|2|<id>|<preimage_hex> settled, with the Lightning preimage
 *
 * `⚡PAYDONE|1|<id>` from old peers is accepted. Unknown versions are not
 * decoded and therefore render as plain text. A DONE may arrive before its PAY;
 * folding the whole channel makes that order-independent. Buzz has no wallet,
 * so it never pays and never checks a preimage against a payment hash: it only
 * shows what the payer (a human or an agent with its own wallet) reported.
 */

export type SonarPayLine =
  | { type: "pay"; id: string; sats: number }
  | { type: "done"; id: string; preimage?: string };

export type SonarPaySettlement = {
  /** Lowercase hex preimage when the payer's wallet returned one. */
  preimage?: string;
};

/** Renderable state of a `⚡PAY` row, attached by the timeline formatter. */
export type SonarPayView = {
  id: string;
  sats: number;
  settled: boolean;
  preimage?: string;
};

const PAY = "⚡PAY";
const PAYDONE = "⚡PAYDONE";
const HEX64 = /^[0-9a-fA-F]{64}$/;

/** Mirror of Sonar's `PayLine.decode`. Returns null for anything else. */
export function decodeSonarPayLine(content: string): SonarPayLine | null {
  const parts = content.split("|");
  if (parts.length < 3) return null;
  const [tag, version, id] = parts;
  if (!id) return null;
  if (tag === PAY) {
    if (version !== "1") return null;
    const raw = parts[3];
    if (raw === undefined || !/^[0-9]+$/.test(raw)) return null;
    const sats = Number(raw);
    if (!Number.isSafeInteger(sats) || sats <= 0) return null;
    return { type: "pay", id, sats };
  }
  if (tag === PAYDONE) {
    if (version === "1") {
      return parts.length === 3 ? { type: "done", id } : null;
    }
    if (version === "2") {
      if (parts.length === 3) return { type: "done", id };
      if (parts.length === 4 && HEX64.test(parts[3])) {
        return { type: "done", id, preimage: parts[3].toLowerCase() };
      }
    }
  }
  return null;
}

/**
 * A message's Sonar payment lines, split from the rest of its text.
 *
 * Sonar sends each line as an entire message. An agent (goose with the
 * `buzz-lightning-mcp` server, for example) usually wraps them in a sentence,
 * so Buzz also accepts them as lines inside a longer message: each receipt
 * line must sit on its own line, with no leading space. Trailing whitespace
 * and `\r` are ignored.
 */
export type SonarPayContent = {
  /** The message with every payment line removed, trimmed. */
  text: string;
  /** `⚡PAY` lines, in order. */
  pays: Array<{ id: string; sats: number }>;
  /** `⚡PAYDONE` lines, in order. */
  dones: Array<{ id: string; preimage?: string }>;
};

export function parseSonarPayContent(content: string): SonarPayContent | null {
  const pays: SonarPayContent["pays"] = [];
  const dones: SonarPayContent["dones"] = [];
  const kept: string[] = [];
  for (const rawLine of content.split("\n")) {
    const line = decodeSonarPayLine(rawLine.replace(/\s+$/, ""));
    if (line?.type === "pay") pays.push({ id: line.id, sats: line.sats });
    else if (line?.type === "done")
      dones.push({ id: line.id, preimage: line.preimage });
    else kept.push(rawLine);
  }
  if (pays.length === 0 && dones.length === 0) return null;
  return { text: kept.join("\n").trim(), pays, dones };
}

/**
 * A message made only of `⚡PAYDONE` lines is a control row: Sonar hides it
 * from the transcript and from unread counts. It only settles a `⚡PAY`.
 */
export function isSonarPayControlLine(content: string): boolean {
  const parsed = parseSonarPayContent(content);
  return (
    parsed !== null &&
    parsed.pays.length === 0 &&
    parsed.dones.length > 0 &&
    parsed.text === ""
  );
}

function settlementKey(authorPubkey: string, id: string) {
  return `${authorPubkey.toLowerCase()}:${id}`;
}

/**
 * Collect settlements from every `⚡PAYDONE` line, keyed by signer and id.
 *
 * Sonar is 1:1, so any DONE settles the matching PAY. A Buzz channel has many
 * writers, so a DONE only settles a PAY signed by the same key: nobody can
 * mark someone else's receipt paid. A DONE carrying a preimage wins over one
 * without, regardless of arrival order.
 */
export function collectSonarPaySettlements(
  events: ReadonlyArray<{ pubkey: string; content: string }>,
): Map<string, SonarPaySettlement> {
  const settlements = new Map<string, SonarPaySettlement>();
  for (const event of events) {
    const parsed = parseSonarPayContent(event.content);
    if (!parsed) continue;
    for (const done of parsed.dones) {
      const key = settlementKey(event.pubkey, done.id);
      const existing = settlements.get(key);
      if (!existing || (!existing.preimage && done.preimage)) {
        settlements.set(key, { preimage: done.preimage });
      }
    }
  }
  return settlements;
}

/** Text plus payment bubbles for a message that carries `⚡PAY` lines. */
export type SonarPayMessageView = {
  /** Remaining message text, rendered above the bubbles (may be empty). */
  text: string;
  receipts: SonarPayView[];
};

/**
 * Bubble state for a message with at least one `⚡PAY` line, or `undefined`
 * for any other message (including DONE-only control rows, which are hidden,
 * and text with only DONE lines, which renders as text without them).
 */
export function resolveSonarPayMessage(
  content: string,
  signerPubkey: string,
  settlements: ReadonlyMap<string, SonarPaySettlement>,
): SonarPayMessageView | undefined {
  const parsed = parseSonarPayContent(content);
  if (!parsed || (parsed.pays.length === 0 && parsed.text === "")) {
    return undefined;
  }
  return {
    text: parsed.text,
    receipts: parsed.pays.map((pay) => {
      const settlement = settlements.get(settlementKey(signerPubkey, pay.id));
      return {
        id: pay.id,
        sats: pay.sats,
        settled: settlement !== undefined,
        preimage: settlement?.preimage,
      };
    }),
  };
}

/** Short plain-text summary for previews and screen readers. */
export function describeSonarPay(view: SonarPayView): string {
  const amount = `${view.sats.toLocaleString("en-US")} sats`;
  return view.settled ? `Paid ${amount}` : `Sending ${amount}`;
}

/**
 * One-line text for previews that show raw message content: notifications,
 * inbox snippets, search results. Receipt lines become "⚡ Paid 21 sats" (or
 * "⚡ 21 sats payment" until a `⚡PAYDONE` in the same message settles it), the
 * rest of the text is kept, and a `⚡PAYDONE`-only control message returns
 * `null` so callers can skip it. Other messages come back unchanged.
 */
export function sonarPayPreviewText(content: string): string | null {
  const parsed = parseSonarPayContent(content);
  if (!parsed) return content;
  if (parsed.pays.length === 0) {
    return parsed.text === "" ? null : parsed.text;
  }
  const settled = new Set(parsed.dones.map((done) => done.id));
  const summaries = parsed.pays.map((pay) => {
    const amount = `${pay.sats.toLocaleString("en-US")} sats`;
    return settled.has(pay.id) ? `⚡ Paid ${amount}` : `⚡ ${amount} payment`;
  });
  return [parsed.text, ...summaries].filter((part) => part !== "").join(" ");
}
