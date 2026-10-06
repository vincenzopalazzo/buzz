import { Check, Copy, Zap } from "lucide-react";

import {
  describeSonarPay,
  type SonarPayView,
} from "@/features/messages/lib/sonarPay";
import { cn } from "@/shared/lib/cn";
import { useCopyFeedback } from "@/shared/ui/HoverCopyIndicator";

type SonarPayBubbleProps = {
  pay: SonarPayView;
  /** True when the current user signed the `⚡PAY` line. */
  mine: boolean;
  className?: string;
};

function statusLabel(pay: SonarPayView, mine: boolean) {
  if (pay.settled) return mine ? "Paid" : "Received";
  return mine ? "Sending" : "Incoming payment";
}

/**
 * Sonar's gold payment receipt bubble (`PayBubble` in SonarPayViews.kt),
 * rebuilt for Buzz. It renders what the payer reported; Buzz holds no wallet
 * and never pays. The preimage, when present, is shown as a copyable proof
 * the payee can check against the payment hash in their own wallet.
 */
export function SonarPayBubble({ pay, mine, className }: SonarPayBubbleProps) {
  const { copied, copy } = useCopyFeedback({
    label: "Preimage",
    value: pay.preimage ?? "",
  });
  const summary = describeSonarPay(pay);
  const label = statusLabel(pay, mine);

  return (
    <div
      className={cn("flex max-w-sm flex-col items-start gap-1", className)}
      data-sonar-pay-state={pay.settled ? "settled" : "pending"}
      data-testid="sonar-pay-bubble"
    >
      {/* One screen-reader stop for the whole receipt; the visual parts are hidden. */}
      <span className="sr-only">{`Lightning payment: ${summary}`}</span>
      <div
        aria-hidden="true"
        className={cn(
          "flex min-w-[12rem] items-center gap-3 rounded-2xl bg-amber-400/90 px-3 py-3 text-amber-950 dark:bg-amber-400/85",
          mine ? "rounded-br-md" : "rounded-bl-md",
        )}
      >
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-amber-200 text-lg font-extrabold text-amber-700 shadow-inner">
          ₿
        </span>
        <div className="flex items-baseline gap-1">
          <span className="text-xl font-extrabold tabular-nums">
            {pay.sats.toLocaleString("en-US")}
          </span>
          <span className="text-xs font-bold opacity-70">sats</span>
        </div>
      </div>
      <div className="flex items-center gap-1 px-1 text-2xs text-muted-foreground">
        <Zap aria-hidden="true" className="h-3 w-3 shrink-0" />
        <span aria-hidden="true">{label}</span>
        {pay.settled && pay.preimage ? (
          <>
            <span aria-hidden="true">·</span>
            <button
              aria-label="Copy payment preimage"
              className="inline-flex items-center gap-1 rounded hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => void copy()}
              title={pay.preimage}
              type="button"
            >
              <span>proof</span>
              {copied ? (
                <Check aria-hidden="true" className="h-3 w-3" />
              ) : (
                <Copy aria-hidden="true" className="h-3 w-3" />
              )}
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
