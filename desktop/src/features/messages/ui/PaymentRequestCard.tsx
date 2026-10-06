import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy, QrCode, Zap } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  derivePaymentState,
  formatSats,
  type ParsedPaymentRequest,
  type PaymentReceiptSummary,
  parsePaymentRequestTags,
  paymentTargetLabel,
  paymentTargetUri,
  verifyPaymentReceipt,
} from "@/features/messages/lib/payment";
import type { TimelineMessage } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { useCopyFeedback } from "@/shared/ui/HoverCopyIndicator";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";

type PaymentRequestCardProps = {
  message: TimelineMessage;
  className?: string;
};

function useNowSeconds(expiry: number | undefined) {
  const [now, setNow] = React.useState(() => Math.floor(Date.now() / 1000));
  React.useEffect(() => {
    if (expiry === undefined || expiry <= now) return undefined;
    const timer = window.setTimeout(
      () => setNow(Math.floor(Date.now() / 1000)),
      Math.min((expiry - now) * 1000 + 500, 60_000),
    );
    return () => window.clearTimeout(timer);
  }, [expiry, now]);
  return now;
}

/**
 * Receipts whose preimage hashes to their payment_hash. Verification runs in
 * the webview with Web Crypto; nothing is trusted from the event itself.
 */
function useVerifiedReceiptIds(receipts: readonly PaymentReceiptSummary[]) {
  const [verified, setVerified] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  React.useEffect(() => {
    let cancelled = false;
    void (async () => {
      const ids = new Set<string>();
      for (const receipt of receipts) {
        if (receipt.status !== "paid" || !receipt.preimage) continue;
        if (await verifyPaymentReceipt(receipt)) ids.add(receipt.id);
      }
      if (!cancelled) setVerified(ids);
    })();
    return () => {
      cancelled = true;
    };
  }, [receipts]);
  return verified;
}

function formatExpiry(expiry: number, now: number): string {
  const remaining = expiry - now;
  if (remaining <= 0) return "Expired";
  if (remaining < 60) return `Expires in ${remaining}s`;
  if (remaining < 3_600) return `Expires in ${Math.ceil(remaining / 60)} min`;
  if (remaining < 86_400) return `Expires in ${Math.ceil(remaining / 3_600)} h`;
  return `Expires in ${Math.ceil(remaining / 86_400)} d`;
}

function TargetRow({
  target,
}: {
  target: ParsedPaymentRequest["targets"][number];
}) {
  const label = paymentTargetLabel(target.type);
  const { copied, copy } = useCopyFeedback({ label, value: target.value });
  const short =
    target.value.length > 40
      ? `${target.value.slice(0, 22)}…${target.value.slice(-12)}`
      : target.value;
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="shrink-0 rounded-md border border-border/60 px-1.5 py-0.5 text-2xs uppercase tracking-[0.14em] text-muted-foreground">
        {label}
      </span>
      <span
        className="min-w-0 truncate font-mono text-xs text-foreground/80"
        title={target.value}
      >
        {short}
      </span>
      <Button
        aria-label={`Copy ${label.toLowerCase()}`}
        className="ml-auto h-6 w-6 shrink-0"
        onClick={() => void copy()}
        size="icon"
        variant="ghost"
      >
        {copied ? (
          <Check className="h-3.5 w-3.5" />
        ) : (
          <Copy className="h-3.5 w-3.5" />
        )}
      </Button>
    </div>
  );
}

/**
 * NIP-LP payment request card (kind 40009).
 *
 * Renders the amount, memo, pay targets and the state derived from the
 * receipts joined to this row. "Verified" appears only when a receipt's
 * preimage hashes to the request's own payment_hash; everything else is the
 * payer's claim. Buzz never pays: "Pay" hands the target to the OS wallet.
 */
export function PaymentRequestCard({
  message,
  className,
}: PaymentRequestCardProps) {
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(cardRef);
  const [showQr, setShowQr] = React.useState(false);

  const request = React.useMemo(
    () => parsePaymentRequestTags(message.tags),
    [message.tags],
  );
  const receipts = message.paymentReceipts ?? [];
  const now = useNowSeconds(request?.expiry);
  const verifiedIds = useVerifiedReceiptIds(receipts);

  if (!request) {
    // Malformed card: fall back to the plain-text content the sender wrote.
    return (
      <p className={cn("whitespace-pre-wrap text-message", className)}>
        {message.body}
      </p>
    );
  }

  const state = derivePaymentState(request, receipts, now, verifiedIds);
  const primaryTarget =
    request.targets.find((t) => t.type === "bolt11") ??
    request.targets.find((t) => t.type === "bolt12") ??
    request.targets[0];
  const qrTarget = request.targets.find(
    (t) => t.type === "bolt11" || t.type === "bolt12",
  );
  const payable = state.kind === "pending" || state.kind === "failed";

  const badge = (() => {
    switch (state.kind) {
      case "pending":
        return <Badge variant="info">Pending</Badge>;
      case "expired":
        return <Badge variant="secondary">Expired</Badge>;
      case "failed":
        return <Badge variant="warning">Failed</Badge>;
      case "paid":
        return state.verified ? (
          <Badge variant="success">Verified</Badge>
        ) : (
          <Badge variant="success">Paid</Badge>
        );
    }
  })();

  const openInWallet = () => {
    if (!primaryTarget) return;
    void openUrl(paymentTargetUri(primaryTarget)).catch(() => {
      toast.error("No Lightning wallet is registered to open this request.");
    });
  };

  return (
    <div
      ref={cardRef}
      className={cn(
        "max-w-md overflow-hidden rounded-2xl border border-border/70 bg-card/60 text-sm",
        className,
      )}
      data-payment-state={
        state.kind === "paid" && state.verified ? "verified" : state.kind
      }
      data-testid="payment-request-card"
    >
      <div className="flex items-center gap-2 border-b border-border/50 bg-muted/40 px-3 py-2">
        <Zap className="h-4 w-4 shrink-0 text-amber-500" />
        <span className="text-xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Payment request
        </span>
        <div className="ml-auto flex items-center gap-2">{badge}</div>
      </div>
      <div className="flex flex-col gap-3 px-3 py-3">
        <div className="flex items-baseline gap-2">
          <span className="text-xl font-semibold tabular-nums text-foreground">
            {formatSats(request.amountMsat)}
          </span>
          {request.expiry !== undefined && state.kind !== "paid" && (
            <span className="text-2xs text-muted-foreground">
              {formatExpiry(request.expiry, now)}
            </span>
          )}
        </div>
        {request.memo && (
          <p className="whitespace-pre-wrap text-foreground/90">
            {request.memo}
          </p>
        )}
        <div className="flex flex-col gap-1.5">
          {request.targets.map((target) => (
            <TargetRow key={`${target.type}:${target.value}`} target={target} />
          ))}
        </div>
        {showQr && qrTarget && (
          <div className="flex justify-center rounded-xl bg-white p-3">
            <StyledQrCode
              size={200}
              title={`${paymentTargetLabel(qrTarget.type)} QR code`}
              value={qrTarget.value.toUpperCase()}
            />
          </div>
        )}
        <div className="flex items-center gap-2">
          {payable && primaryTarget && (
            <Button
              data-testid="payment-request-pay"
              onClick={openInWallet}
              size="sm"
            >
              <Zap className="mr-1 h-3.5 w-3.5" />
              Pay with wallet
            </Button>
          )}
          {qrTarget && (
            <Button
              onClick={() => setShowQr((v) => !v)}
              size="sm"
              variant="outline"
            >
              <QrCode className="mr-1 h-3.5 w-3.5" />
              {showQr ? "Hide QR" : "Show QR"}
            </Button>
          )}
        </div>
        {receipts.length > 0 && (
          <ul className="flex flex-col gap-1 border-t border-border/50 pt-2 text-xs text-muted-foreground">
            {receipts.map((receipt) => (
              <li
                key={receipt.id}
                className="flex items-center gap-1.5"
                data-testid="payment-receipt"
              >
                {receipt.status === "paid" ? (
                  <Check className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                ) : (
                  <Zap className="h-3.5 w-3.5 shrink-0 text-amber-500" />
                )}
                <span className="min-w-0 truncate">
                  {receipt.status === "paid"
                    ? `${receipt.payerDisplayName} paid ${formatSats(receipt.amountMsat)}`
                    : `${receipt.payerDisplayName}: payment failed${
                        receipt.reason ? ` — ${receipt.reason}` : ""
                      }`}
                  {receipt.status === "paid" &&
                    (verifiedIds.has(receipt.id) &&
                    request.paymentHash === receipt.paymentHash
                      ? " · preimage verified"
                      : " · unverified")}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
