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
 * `⚡PAYDONE` lines are control rows: Sonar hides them from the transcript and
 * from unread counts. They only change the state of the matching `⚡PAY`.
 */
export function isSonarPayControlLine(content: string): boolean {
  return decodeSonarPayLine(content)?.type === "done";
}

function settlementKey(authorPubkey: string, id: string) {
  return `${authorPubkey.toLowerCase()}:${id}`;
}

/**
 * Collect settlements from `⚡PAYDONE` messages, keyed by signer and id.
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
    const line = decodeSonarPayLine(event.content);
    if (line?.type !== "done") continue;
    const key = settlementKey(event.pubkey, line.id);
    const existing = settlements.get(key);
    if (!existing || (!existing.preimage && line.preimage)) {
      settlements.set(key, { preimage: line.preimage });
    }
  }
  return settlements;
}

/** The bubble state for a message whose content is a `⚡PAY` line. */
export function resolveSonarPayView(
  content: string,
  signerPubkey: string,
  settlements: ReadonlyMap<string, SonarPaySettlement>,
): SonarPayView | undefined {
  const line = decodeSonarPayLine(content);
  if (line?.type !== "pay") return undefined;
  const settlement = settlements.get(settlementKey(signerPubkey, line.id));
  return {
    id: line.id,
    sats: line.sats,
    settled: settlement !== undefined,
    preimage: settlement?.preimage,
  };
}

/** Short plain-text summary for previews and screen readers. */
export function describeSonarPay(view: SonarPayView): string {
  const amount = `${view.sats.toLocaleString("en-US")} sats`;
  return view.settled ? `Paid ${amount}` : `Sending ${amount}`;
}
