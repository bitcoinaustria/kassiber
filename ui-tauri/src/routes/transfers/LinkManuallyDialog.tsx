/**
 * "Link manually": one screen with the two sides of a transfer on top, the
 * send and the receipt. While a side is open, the list under them offers
 * the book's transactions for it, ranked against the other side. Once both
 * are chosen, the same place shows the check: what the two differ by, what
 * the link counts as and the core's dry run of the book with the link in
 * it. One confirmation applies exactly that run.
 *
 * What a link counts as is stated, not chosen, wherever the core records it
 * one way only (the same asset on both sides: the owner's own coins moving).
 * Between two assets the core records either that or a sale and a purchase,
 * so the owner is asked one question with both consequences spelled out.
 * The kind of move (a peg, a Lightning swap) is read from the two sides.
 *
 * Most of the time one side is already known (a suggestion, a row, a deep
 * link, "Link again"), so the usual path is one pick and one confirmation.
 * It works for any two transactions the core accepts as a pair, including a
 * pair the matcher never suggests. Nothing is linked before the owner
 * confirms, and nothing here decides accounting: the core plans the link,
 * books it and refuses the ones it cannot.
 *
 * The same content opens two ways: as a dialog over the page, or as a panel
 * docked beside the page's lists (rotki's "Pin section"), so the owner can
 * keep looking through the page while linking. The content is one tree that
 * moves between the two frames, so a link in progress (the picks, the
 * search, the answer, a running check) survives the page getting wider or
 * narrower.
 *
 * A send that arrived in several receipts (or several sends into one
 * receipt) is linked part by part: a transaction whose links leave part of
 * it open can be picked again, and the check is about the part that is left.
 */
import * as React from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, ArrowRight, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { blurClass } from "@/components/transactions/model";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { candidateTerms, isBitcoinAsset, type TransferSuggestions } from "@/components/review/transferModel";
import { useDaemon } from "@/daemon/client";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { LinkCandidates } from "./LinkCandidates";
import {
  bpsText,
  defaultPairKind,
  differenceOf,
  hasOpenPart,
  linkAnswer,
  sharesWith,
  linkQuestion,
  pairWarnings,
  relinkPortion,
  withOpenPart,
  type Difference,
  type LegSide,
  type LinkAnswer,
  type LinkLeg,
} from "./linkModel";
import { PreviewOutcomeView } from "./MoveEffects";
import { MoveLeg } from "./MoveLeg";
import { appliedId, daemonErrorCode, moveOutcome, useReviewPreview, type ReviewOperation } from "./reviewPreview";
import { formatMsatAsBtc, formatMsatAsSats } from "./satsFormat";
import { useMoveLeg } from "./useMoveLeg";
import { useOpenParts } from "./useOpenParts";

/** Stored with the pair; the book keeps one language, like other stored reasons. */
export const MANUAL_LINK_REASON = "Linked by hand on Transfers";
export const RELINK_REASON = "Linked again after unlinking";

export interface LinkStart {
  /** Start from this transaction: a send picks its receipt, a receipt its send. */
  transactionId?: string;
  /** Both sides known: open on the check. */
  sendId?: string;
  receiptId?: string;
  /** Terms to propose, from the pair as it was before it was unlinked. */
  preset?: { kind?: string | null; policy?: string | null; outAmountMsat?: string | number | null };
  relink?: boolean;
}

/** What the page needs once a link is made. */
export interface LinkedResult {
  /** The new pair, for Undo; null when the core did not name it. */
  pairId: string | null;
  sendId: string;
  receiptId: string;
}

/** Which ways the book lets two transactions be linked (`ui.transfers.pair_options`). */
interface PairOptions {
  policies?: Array<{ policy: string; allowed: boolean; reason?: string | null; message?: string | null }>;
}

export type LinkVariant = "dialog" | "panel";

type Translate = ReturnType<typeof useTranslation<"review">>["t"];

export function LinkManuallyDialog({
  start,
  variant = "dialog",
  panelSlot,
  updating = false,
  onClose,
  onLinked,
}: {
  /** Null keeps the dialog closed. */
  start: LinkStart | null;
  /** A dialog over the page, or a panel that stays beside its lists. */
  variant?: LinkVariant;
  /** Where on the page the panel sits; without one it renders in place. */
  panelSlot?: HTMLElement | null;
  /** The journals are recalculating: the check waits for them. */
  updating?: boolean;
  onClose: () => void;
  /** The link is made; the dialog has nothing more to show. */
  onLinked: (result: LinkedResult) => void;
}) {
  if (!start) return variant === "panel" ? null : <Dialog open={false} />;
  return (
    <LinkManuallyFlow
      key={JSON.stringify(start)}
      start={start}
      variant={variant}
      panelSlot={panelSlot}
      updating={updating}
      onClose={onClose}
      onLinked={onLinked}
    />
  );
}

/**
 * The frame around the flow: a dialog, or a panel beside the page's lists.
 * Its content sizes itself by this frame (`@container/link`), so the narrow
 * panel stacks what the dialog sets side by side.
 */
function LinkShell({
  variant,
  panelSlot,
  title,
  description,
  state,
  busy,
  onClose,
  attach,
  children,
}: {
  variant: LinkVariant;
  panelSlot?: HTMLElement | null;
  title: string;
  description: string;
  state: "pick" | "check";
  busy: boolean;
  onClose: () => void;
  /** Takes the element the flow's content is moved into. */
  attach: (node: HTMLElement | null) => void;
  /** The content itself, only where it cannot be moved (no document). */
  children?: React.ReactNode;
}) {
  const { t } = useTranslation("review");
  const headingRef = React.useRef<HTMLHeadingElement>(null);
  const shown = variant === "panel" && panelSlot !== null;
  // A panel takes no focus trap, so it says it opened by taking focus once.
  React.useEffect(() => {
    if (shown) headingRef.current?.focus();
  }, [shown]);
  if (variant === "panel") {
    // The page's place for the panel is not there yet.
    if (panelSlot === null) return null;
    const panel = (
      <aside
        className="kb-surface @container/link flex max-h-[var(--kb-panel-room,calc(100vh-6rem))] flex-col gap-4 overflow-x-hidden overflow-y-auto p-4 text-sm"
        aria-labelledby="link-panel-title"
        data-testid="link-manually"
        data-variant="panel"
        data-state={state}
        onKeyDown={(event) => {
          if (event.key !== "Escape" || busy || event.defaultPrevented) return;
          event.stopPropagation();
          onClose();
        }}
      >
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1 space-y-1">
            <h2 id="link-panel-title" ref={headingRef} tabIndex={-1} className="rounded-sm text-base leading-tight font-semibold focus-visible:outline-none">
              {title}
            </h2>
            <p className="text-muted-foreground">{description}</p>
          </div>
          <Button type="button" size="icon" variant="ghost" className="size-7 shrink-0" disabled={busy} aria-label={t("transfersPage.unlink.close")} onClick={onClose}>
            <X className="size-4" aria-hidden="true" />
          </Button>
        </div>
        <div ref={attach} className="contents">
          {children}
        </div>
      </aside>
    );
    return panelSlot ? createPortal(panel, panelSlot) : panel;
  }
  return (
    <Dialog open onOpenChange={(open) => (!open && !busy ? onClose() : null)}>
      <DialogContent
        className="@container/link flex max-h-[90vh] flex-col gap-4 overflow-x-hidden overflow-y-auto sm:max-w-3xl"
        data-testid="link-manually"
        data-variant="dialog"
        data-state={state}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <div ref={attach} className="contents">
          {children}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function LinkManuallyFlow({
  start,
  variant,
  panelSlot,
  updating,
  onClose,
  onLinked,
}: {
  start: LinkStart;
  variant: LinkVariant;
  panelSlot?: HTMLElement | null;
  updating: boolean;
  onClose: () => void;
  onLinked: (result: LinkedResult) => void;
}) {
  const { t } = useTranslation("review");
  // The content lives in one element of its own that the frame in use takes
  // in, so changing the frame moves it instead of starting it over.
  const [host] = React.useState(() => {
    if (typeof document === "undefined") return null;
    const element = document.createElement("div");
    element.className = "contents";
    return element;
  });
  const attach = React.useCallback((node: HTMLElement | null) => (node && host ? void node.appendChild(host) : undefined), [host]);
  // The panel takes focus when it opens and gives it back to where it came
  // from when it goes; a dialog does that itself.
  const docked = React.useRef(variant === "panel");
  React.useEffect(() => {
    docked.current = variant === "panel";
  }, [variant]);
  // Read before the panel takes focus for itself.
  const [opener] = React.useState(() => (typeof document === "undefined" ? null : document.activeElement));
  React.useEffect(() => {
    return () => {
      if (docked.current && opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  }, [opener]);
  const both = Boolean(start.sendId && start.receiptId);
  const startTx = useMoveLeg(start.transactionId && !both ? start.transactionId : null);
  const [sendId, setSendId] = React.useState<string | null>(start.sendId ?? null);
  const [receiptId, setReceiptId] = React.useState<string | null>(start.receiptId ?? null);
  // The side whose list is open; null shows the check.
  const [picking, setPicking] = React.useState<LegSide | null>(both ? null : (start.receiptId ? "send" : start.sendId ? "receipt" : "send"));
  const [notice, setNotice] = React.useState<"unknown" | "gone" | null>(null);
  const [applying, setApplying] = React.useState(false);

  // A starting transaction takes its own side once it is read.
  const placed = React.useRef(false);
  React.useEffect(() => {
    if (placed.current || startTx.loading || !start.transactionId || both) return;
    placed.current = true;
    if (!startTx.leg) {
      // Not a transaction of this book: both sides are open.
      setNotice("unknown");
    } else if (startTx.leg.side === "send") {
      setSendId(startTx.leg.id);
      setPicking("receipt");
    } else {
      setReceiptId(startTx.leg.id);
      setPicking("send");
    }
  }, [both, start.transactionId, startTx.leg, startTx.loading]);

  // A linked transaction with a part left open can take another link.
  const parts = useOpenParts();
  const sendRead = useMoveLeg(sendId);
  const receiptRead = useMoveLeg(receiptId);
  const send = React.useMemo(() => ({ ...sendRead, leg: sendRead.leg ? withOpenPart(sendRead.leg, parts) : null }), [parts, sendRead]);
  const receipt = React.useMemo(
    () => ({ ...receiptRead, leg: receiptRead.leg ? withOpenPart(receiptRead.leg, parts) : null }),
    [parts, receiptRead],
  );
  // A chosen transaction that the book no longer has opens its side again.
  React.useEffect(() => {
    if (sendId && !send.loading && !send.leg) {
      setSendId(null);
      setPicking("send");
      setNotice("gone");
    } else if (receiptId && !receipt.loading && !receipt.leg) {
      setReceiptId(null);
      setPicking((current) => current ?? "receipt");
      setNotice("gone");
    }
  }, [receipt.leg, receipt.loading, receiptId, send.leg, send.loading, sendId]);

  const reading = Boolean(start.transactionId && !both && startTx.loading);
  // What the list knew about a pick, shown until the transaction itself is read.
  const [picked, setPicked] = React.useState<Partial<Record<LegSide, LinkLeg>>>({});
  const choose = (side: LegSide, leg: LinkLeg) => {
    setNotice(null);
    setPicked((current) => ({ ...current, [side]: leg }));
    if (side === "send") {
      setSendId(leg.id);
      setPicking(receiptId ? null : "receipt");
    } else {
      setReceiptId(leg.id);
      setPicking(sendId ? null : "send");
    }
  };
  // The other side as the list knew it, until it is read: the candidates rank against it at once.
  const known = (side: LegSide) => {
    const state = side === "send" ? send : receipt;
    const id = side === "send" ? sendId : receiptId;
    return state.leg ?? (id && picked[side]?.id === id ? (picked[side] ?? null) : null);
  };
  const ready = !picking && send.leg && receipt.leg ? { send: send.leg, receipt: receipt.leg } : null;
  // A side chosen from outside may already be linked or ignored: say so before the other is picked.
  const early = picking
    ? [send.leg, receipt.leg].flatMap((leg) =>
        leg && (leg.excluded || (leg.linked && !hasOpenPart(leg)))
          ? [warningText(t, leg.side === "send" ? (leg.excluded ? "sendIgnored" : "sendLinked") : leg.excluded ? "receiptIgnored" : "receiptLinked", leg, leg)]
          : [],
      )
    : [];

  const slot = (side: LegSide) => {
    const state = side === "send" ? send : receipt;
    const id = side === "send" ? sendId : receiptId;
    const label = t(side === "send" ? "transfersPage.leg.sent" : "transfersPage.leg.received");
    const active = picking === side;
    const listed = picked[side]?.id === id ? picked[side] : undefined;
    if (!id) {
      return (
        <button
          type="button"
          aria-pressed={active}
          disabled={reading}
          onClick={() => setPicking(side)}
          data-testid="link-slot"
          data-side={side}
          className={cn(
            "flex min-h-[4.5rem] min-w-0 flex-col justify-center gap-1 rounded-lg border border-dashed px-3 py-2.5 text-left transition-colors hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:cursor-wait",
            active && "border-solid border-primary bg-primary/5",
          )}
        >
          <span className="text-2xs font-medium tracking-wide text-muted-foreground uppercase sm:text-xs">{label}</span>
          <span className={cn("text-sm", !active && "text-muted-foreground")}>
            {reading
              ? t("transfersPage.link.reading")
              : t(
                  active
                    ? side === "send"
                      ? "transfersPage.link.slot.pickSend"
                      : "transfersPage.link.slot.pickReceipt"
                    : side === "send"
                      ? "transfersPage.link.slot.openSend"
                      : "transfersPage.link.slot.openReceipt",
                )}
          </span>
        </button>
      );
    }
    return (
      <MoveLeg
        label={label}
        leg={state.leg}
        loading={state.loading}
        fallback={listed ? { wallet: listed.wallet, rail: listed.rail, amountMsat: listed.amountMsat, occurredAt: listed.occurredAt } : undefined}
        onOpen={onClose}
        className={cn(active && "border-primary bg-primary/5")}
        action={
          active || applying ? null : (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-6 px-2 text-xs"
              aria-label={t(side === "send" ? "transfersPage.link.changeSend" : "transfersPage.link.changeReceipt")}
              onClick={() => setPicking(side)}
            >
              {t("transfersPage.link.change")}
            </Button>
          )
        }
      />
    );
  };

  const content = (
    <>
      {notice ? (
        <p className="rounded-md border bg-muted/30 px-3 py-2 text-sm" role="status" data-testid="link-notice">
          {t(`transfersPage.link.notice.${notice}`)}
        </p>
      ) : null}
      <div className="grid grid-cols-1 items-stretch gap-2 @xl/link:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]">
        {slot("send")}
        <span className="hidden place-items-center text-muted-foreground @xl/link:grid">
          <ArrowRight className="size-4" aria-hidden="true" />
        </span>
        {slot("receipt")}
      </div>
      {early.length ? <Warnings lines={early} /> : null}
      {reading ? null : picking ? (
        <>
          <LinkCandidates
            // A new list for the other side, or for another transaction to compare with.
            key={`${picking}:${known(picking === "send" ? "receipt" : "send")?.id ?? ""}`}
            side={picking}
            anchor={known(picking === "send" ? "receipt" : "send")}
            parts={parts}
            selectedId={picking === "send" ? sendId : receiptId}
            onPick={(leg) => choose(picking, leg)}
            onOpen={onClose}
          />
          <DialogFooter className="gap-2 sm:justify-between">
            {sendId && receiptId ? (
              <Button type="button" variant="ghost" onClick={() => setPicking(null)}>
                {t(picking === "send" ? "transfersPage.link.keepSend" : "transfersPage.link.keepReceipt")}
              </Button>
            ) : (
              <span />
            )}
            <Button type="button" variant="outline" onClick={onClose}>
              {t("transfersPage.link.cancel")}
            </Button>
          </DialogFooter>
        </>
      ) : ready ? (
        <CheckAndLink
          key={`${ready.send.id}>${ready.receipt.id}`}
          send={ready.send}
          receipt={ready.receipt}
          preset={start.preset}
          relink={Boolean(start.relink)}
          updating={updating}
          onApplying={setApplying}
          onCancel={onClose}
          onLinked={onLinked}
        />
      ) : (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          {t("transfersPage.link.reading")}
        </p>
      )}
    </>
  );
  return (
    <>
      <LinkShell
        variant={variant}
        panelSlot={panelSlot}
        title={start.relink ? t("transfersPage.link.titleAgain") : t("transfersPage.link.title")}
        description={t(picking ? "transfersPage.link.subtitle" : "transfersPage.link.subtitleCheck")}
        state={picking ? "pick" : "check"}
        busy={applying}
        onClose={onClose}
        attach={attach}
      >
        {host ? null : content}
      </LinkShell>
      {host
        ? createPortal(
            // Events reach React through this tree, not through the frame: Escape closes the panel from here.
            <div
              className="contents"
              onKeyDown={(event) => {
                if (variant !== "panel" || event.key !== "Escape" || applying || event.defaultPrevented) return;
                event.stopPropagation();
                onClose();
              }}
            >
              {content}
            </div>,
            host,
          )
        : null}
    </>
  );
}

/** What two chosen legs differ by, as one sentence. `part`: the send is linked in part already. */
function differenceSentence(t: Translate, difference: Difference, part = false): string {
  if (difference.kind === "otherAsset") return t("transfersPage.link.diff.otherAssetLong");
  // For the part left of a send, what matters is what stays open after this link.
  if (part && difference.kind === "none") return t("transfersPage.link.partDone");
  if (part && difference.kind !== "negative") {
    return t("transfersPage.link.partAfter", { amount: formatMsatAsBtc(difference.amountMsat.toString()) });
  }
  const amount = formatMsatAsSats(difference.amountMsat.toString());
  const share = difference.shareBps === null ? "—" : difference.shareBps === 0n ? "<0.01" : bpsText(difference.shareBps);
  switch (difference.kind) {
    case "none":
      return t("transfersPage.link.diff.sameLong");
    case "negative":
      return t("transfersPage.link.diff.moreLong", { amount });
    case "mismatch":
      return t("transfersPage.link.diff.mismatchLong", { amount, share });
    default:
      return t("transfersPage.link.diff.lessLong", { amount, share });
  }
}

function warningText(t: Translate, key: ReturnType<typeof pairWarnings>[number]["key"], send: LinkLeg, receipt: LinkLeg): string {
  const leg = key.startsWith("send") ? send : receipt;
  if (key === "sendLinked" || key === "receiptLinked") {
    return leg.link?.wallet ? t(`transfersPage.link.warn.${key}To`, { wallet: leg.link.wallet }) : t(`transfersPage.link.warn.${key}`);
  }
  return t(`transfersPage.link.warn.${key}`);
}

/** Everything to look at again, in one place. */
function Warnings({ lines }: { lines: string[] }) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  return (
    <ul className="space-y-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-900 dark:text-amber-100" data-testid="link-warnings">
      {lines.map((line) => (
        <li key={line} className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          <span className={blurClass(hideSensitive)}>{line}</span>
        </li>
      ))}
    </ul>
  );
}

/** A labelled line of the check: the label in its own column when there is room. */
function CheckRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-x-4 gap-y-1 @xl/link:grid-cols-[7.5rem_minmax(0,1fr)]">
      <dt className="pt-px text-xs font-medium text-muted-foreground @xl/link:text-sm @xl/link:font-normal">{label}</dt>
      <dd className="min-w-0 text-sm">{children}</dd>
    </div>
  );
}

/** The plain reason behind a refusal the core names by code. */
function refusalText(t: Translate, code: string | null): string | null {
  switch (code) {
    case "conflict":
    case "custody_component_membership_conflict":
      return t("transfersPage.link.refused.conflict");
    case "custody_legacy_group_invalid":
    case "custody_legacy_group_ambiguous":
      return t("transfersPage.link.refused.nothingLeft");
    case "transfer_network_mismatch":
      return t("transfersPage.link.refused.network");
    case "not_found":
      return t("transfersPage.link.refused.notFound");
    case "validation":
    case "rail_policy_required":
      return t("transfersPage.link.refused.terms");
    default:
      return null;
  }
}

function CheckAndLink({
  send,
  receipt,
  preset,
  relink,
  updating,
  onApplying,
  onCancel,
  onLinked,
}: {
  send: LinkLeg;
  receipt: LinkLeg;
  preset?: LinkStart["preset"];
  relink: boolean;
  updating: boolean;
  onApplying: (applying: boolean) => void;
  onCancel: () => void;
  onLinked: (result: LinkedResult) => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [error, setError] = React.useState<string | null>(null);
  const question = linkQuestion(send, receipt);
  // What the core itself proposes for exactly this pair, when its matcher suggests it.
  const suggest = useDaemon<TransferSuggestions>("ui.transfers.suggest", {});
  const candidate = suggest.data?.data?.candidates?.find((row) => row.out_id === send.id && row.in_id === receipt.id);
  const suggested = candidate ? candidateTerms(candidate) : null;
  const before = linkAnswer(preset?.policy);
  // Which answers this book allows for these two, asked before any is offered.
  // Unread or unreadable, both stay on offer and the dry run still refuses.
  const allowedRead = useDaemon<PairOptions>("ui.transfers.pair_options", { tx_out: send.id, tx_in: receipt.id }, { enabled: question !== "none", retry: false });
  const policies = question === "none" ? undefined : allowedRead.data?.data?.policies;
  const reading = question !== "none" && allowedRead.isLoading;
  // The one case the core has: between two assets, some books count the move
  // as a sale only. Then that is stated instead of asked. Anything else the
  // read says leaves both answers on offer, and the dry run decides.
  const carryRefused = policies?.find((row) => row.policy === "carrying-value" && !row.allowed) ?? null;
  const saleOnly = Boolean(carryRefused && policies?.some((row) => row.policy === "taxable" && row.allowed));
  // The answer is the owner's. It starts from what the pair had before it was
  // unlinked, or from the core's suggestion; without either, nothing is assumed.
  const [chosen, setChosen] = React.useState<LinkAnswer | null>(null);
  const answer: LinkAnswer | null =
    question === "none" ? "carrying-value" : reading ? null : saleOnly ? "taxable" : (chosen ?? before ?? linkAnswer(suggested?.policy));
  const kind = preset?.kind ?? suggested?.kind ?? defaultPairKind(send, receipt);
  // Only a real part of the send was swapped. One that cannot be sent exactly
  // stops the link: left out, the core would link the whole send.
  const portion = relinkPortion(preset?.outAmountMsat, send.amountMsat, question !== "none");
  const swappedPortion = portion.kind === "part" ? portion.msat : null;
  const portionUnsendable = portion.kind === "unsendable";

  const operations = React.useMemo<ReviewOperation[] | null>(() => {
    if (!answer || portionUnsendable) return null;
    const operation: ReviewOperation = {
      type: "pair",
      out_transaction_id: send.id,
      in_transaction_id: receipt.id,
      pair_kind: kind,
      policy: answer,
      reason: relink ? RELINK_REASON : MANUAL_LINK_REASON,
    };
    if (swappedPortion !== null) operation.out_amount_msat = swappedPortion;
    return [operation];
  }, [answer, kind, portionUnsendable, receipt.id, relink, send.id, swappedPortion]);
  // While the journals recalculate, a plan would be read from a book that is about to change.
  const preview = useReviewPreview(updating ? null : operations, t("transfersPage.effects.unavailable"));
  const outcome = preview.state.status === "ready" ? moveOutcome(preview.state.artifact, send.id, receipt.id) : null;
  const { applying } = preview;
  React.useEffect(() => {
    onApplying(applying);
    return () => onApplying(false);
  }, [applying, onApplying]);

  const difference = differenceOf(send, receipt);
  const warnings = pairWarnings(send, receipt);
  // Holds this link itself creates, as against ones it leaves in place.
  const createsHolds = Boolean(outcome && (outcome.sendHeld || outcome.receiptHeld) && !outcome.wasHeld);
  const lines: string[] = [];
  if (outcome) {
    if (outcome.wasHeld && !outcome.sendHeld && !outcome.receiptHeld) lines.push(t("transfersPage.link.effectReleased"));
    if (createsHolds) {
      lines.push(t("transfersPage.link.effectCreatesHolds", { count: Math.max(1, outcome.heldAfter - outcome.heldBefore) }));
    } else if (outcome.sendHeld || outcome.receiptHeld) {
      lines.push(t("transfersPage.link.effectStillHeld"));
    }
  }
  const anyway = createsHolds || warnings.some((warning) => warning.serious);
  // A transaction linked in part: say what is already linked and what this link is about.
  // Only an ordinary move within one asset can take the rest of a transaction.
  const shares = sharesWith(send, receipt);
  const partNotes = [send, receipt].flatMap((leg) =>
    shares && hasOpenPart(leg) && leg.open
      ? [
          t(leg.side === "send" ? "transfersPage.link.partNoteSend" : "transfersPage.link.partNoteReceipt", {
            linked: formatMsatAsBtc(leg.open.linkedMsat.toString()),
            left: formatMsatAsBtc(leg.open.unlinkedMsat.toString()),
          }),
        ]
      : [],
  );
  // Far less arrived than left, within one asset: the rest may have arrived
  // in another transaction, which the next link can take.
  // in another transaction, which the next link can take. Said in the one
  // warning about the amounts, not beside it.
  const restPossible = difference.kind === "mismatch" && !hasOpenPart(send) && sharesWith(send, receipt) && kind === "manual";

  // One confirmation, however often the button is pressed.
  const sending = React.useRef(false);
  const confirm = async () => {
    if (sending.current || preview.state.status !== "ready") return;
    sending.current = true;
    setError(null);
    try {
      const result = await preview.apply();
      onLinked({ pairId: appliedId(result, "pair_id"), sendId: send.id, receiptId: receipt.id });
    } catch (cause) {
      if (daemonErrorCode(cause) === "review_plan_stale") {
        // The book changed since the check: run it again rather than link on old figures.
        setError(t("transfersPage.link.stale"));
        preview.retry();
      } else {
        const message = cause instanceof Error && cause.message ? cause.message : null;
        const refusal = refusalText(t, daemonErrorCode(cause));
        setError([refusal ?? t("transfersPage.link.failed"), message].filter(Boolean).join(" "));
      }
    } finally {
      sending.current = false;
    }
  };

  const bitcoin = isBitcoinAsset(send.asset) && isBitcoinAsset(receipt.asset);
  const options: Array<{ value: LinkAnswer; label: string; hint: string }> = [
    {
      value: "carrying-value",
      label: t(bitcoin ? "transfersPage.link.question.keepBitcoin" : "transfersPage.link.question.keepAsset"),
      hint: t(bitcoin ? "transfersPage.link.question.keepBitcoinHint" : "transfersPage.link.question.keepAssetHint"),
    },
    { value: "taxable", label: t("transfersPage.link.question.sale"), hint: t("transfersPage.link.question.saleHint") },
  ];
  // Which answer was there before the owner touched anything, and why.
  const proposed = before ?? linkAnswer(suggested?.policy);
  const proposedNote = before ? t("transfersPage.link.question.asBefore") : t("transfersPage.link.suggested");

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4" data-testid="link-check">
      <dl className="space-y-3">
        <CheckRow label={t("transfersPage.link.differenceTitle")}>
          {partNotes.map((note) => (
            <p key={note} className={cn("text-muted-foreground tabular-nums", blurClass(hideSensitive))} data-testid="link-part-note">
              {note}
            </p>
          ))}
          <p className={cn("tabular-nums", blurClass(hideSensitive))} data-testid="link-difference">
            {differenceSentence(t, difference, shares && hasOpenPart(send))}
          </p>
        </CheckRow>
        {question === "none" ? (
          <CheckRow label={t("transfersPage.link.countsAs")}>
            <p data-testid="link-counts-as">{t(bitcoin ? "transfersPage.link.ownBitcoin" : "transfersPage.link.ownAsset")}</p>
          </CheckRow>
        ) : reading ? (
          <CheckRow label={t("transfersPage.link.countsAs")}>
            <p className="flex items-center gap-2 text-muted-foreground" role="status">
              <Loader2 className="size-4 shrink-0 animate-spin" aria-hidden="true" />
              {t("transfersPage.link.reading")}
            </p>
          </CheckRow>
        ) : saleOnly ? (
          // The book allows one answer only: stated with the reason, nothing to choose.
          <CheckRow label={t("transfersPage.link.countsAs")}>
            <div data-testid="link-counts-as" data-answer="taxable">
              <p>{t("transfersPage.link.onlySale")}</p>
              <p className="text-xs text-muted-foreground">
                {t(carryRefused?.reason === "network_mismatch" ? "transfersPage.link.onlyWhy.network" : "transfersPage.link.onlyWhy.crossAsset")}
              </p>
              {swappedPortion !== null ? (
                <p className={cn("mt-1.5 text-xs text-muted-foreground", blurClass(hideSensitive))}>
                  {t("transfersPage.link.portion", { amount: formatMsatAsBtc(swappedPortion) })}
                </p>
              ) : null}
            </div>
          </CheckRow>
        ) : (
          <CheckRow label={t("transfersPage.link.question.title")}>
            <div role="radiogroup" aria-label={t("transfersPage.link.question.title")} className="grid gap-2 @xl/link:grid-cols-2" data-testid="link-question">
              {options.map((option) => {
                const checked = answer === option.value;
                return (
                  <label
                    key={option.value}
                    className={cn(
                      "flex min-w-0 cursor-pointer items-start gap-2 rounded-lg border px-3 py-2 transition-colors hover:bg-muted/40 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                      checked && "border-primary bg-primary/5",
                      applying && "cursor-not-allowed opacity-60",
                    )}
                  >
                    <input
                      type="radio"
                      name="link-answer"
                      className="mt-1 accent-primary"
                      value={option.value}
                      checked={checked}
                      disabled={applying}
                      onChange={() => setChosen(option.value)}
                    />
                    <span className="min-w-0 space-y-0.5">
                      <span className="flex flex-wrap items-center gap-x-2 font-medium">
                        {option.label}
                        {proposed === option.value ? <span className="text-xs font-normal text-muted-foreground">{proposedNote}</span> : null}
                      </span>
                      <span className="block text-xs text-muted-foreground">{option.hint}</span>
                    </span>
                  </label>
                );
              })}
            </div>
            {swappedPortion !== null ? (
              <p className={cn("mt-1.5 text-xs text-muted-foreground", blurClass(hideSensitive))}>
                {t("transfersPage.link.portion", { amount: formatMsatAsBtc(swappedPortion) })}
              </p>
            ) : null}
          </CheckRow>
        )}
        {reading ? null : (
        <CheckRow label={t("transfersPage.link.changesTitle")}>
          {portionUnsendable ? (
            <p className="text-destructive" role="alert" data-testid="link-portion-unsendable">
              {t("transfersPage.link.portionUnsendable")}
            </p>
          ) : answer ? (
            <PreviewOutcomeView
              state={preview.state}
              outcome={outcome}
              lines={lines}
              // The line above already says that reports wait for the holds this link makes.
              reportsSaid={createsHolds}
              waiting={updating ? t("transfersPage.link.waitingJournals") : null}
              refusal={preview.state.status === "failed" ? refusalText(t, preview.state.code) : null}
              onRetry={preview.state.status === "failed" && refusalText(t, preview.state.code) === null ? preview.retry : undefined}
            />
          ) : (
            <p className="text-muted-foreground" data-testid="link-unanswered">
              {t("transfersPage.link.question.unanswered")}
            </p>
          )}
        </CheckRow>
        )}
      </dl>
      {warnings.length ? (
        <Warnings
          lines={warnings.map((warning) =>
            warning.key === "mismatch" && restPossible ? t("transfersPage.link.warn.mismatchOrRest") : warningText(t, warning.key, send, receipt),
          )}
        />
      ) : null}
      {error ? (
        <p className="text-sm break-words text-destructive" role="alert">
          {error}
        </p>
      ) : null}
      <DialogFooter className="gap-2">
        <Button type="button" variant="outline" disabled={applying} onClick={onCancel}>
          {t("transfersPage.link.cancel")}
        </Button>
        <Button
          type="button"
          variant={anyway ? "destructive" : "default"}
          disabled={applying || updating || preview.state.status !== "ready"}
          onClick={() => void confirm()}
        >
          {applying ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
          {applying
            ? t("transfersPage.link.linking")
            : anyway
              ? t("transfersPage.link.confirmAnyway")
              : relink
                ? t("transfersPage.link.confirmAgain")
                : t("transfersPage.link.confirm")}
        </Button>
      </DialogFooter>
    </div>
  );
}
