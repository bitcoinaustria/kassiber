/**
 * Reconcile — address / transaction-id ownership lookup.
 *
 * Paste a pile of addresses or txids (Bitcoin or Liquid, mixed) and see which
 * belong to a wallet in the active profile — naming the wallet and whether it
 * is a receive or change address — and which are external. The reconciliation
 * workflow for telling apart historic payments from transfers between your own
 * wallets. Matching runs locally against synced inventory and offline
 * descriptor derivation; nothing leaves the device. The one exception is the
 * explicit "Verify on chain" action, which asks the default backend about the
 * entries local data cannot settle. Deeper verification is available from the
 * `kassiber wallets identify` CLI.
 *
 * Laid out as a workbench: the entries stay editable on the left while their
 * results fill the right, so a list can be corrected and re-checked in place.
 */
import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  ChevronDown,
  ChevronRight,
  ClipboardCopy,
  FileSpreadsheet,
  Globe,
  Search,
  ShieldCheck,
  Square,
  TriangleAlert,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import { Textarea } from "@/components/ui/textarea";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CopyButton } from "@/components/kb/CopyButton";
import { hiddenSensitiveClassName } from "@/components/kb/wallets/format";
import {
  useDaemon,
  useDaemonMutation,
  useDaemonStreamMutation,
} from "@/daemon/client";
import { makeDaemonRequestId } from "@/daemon/transport";
import { copyTextWithPolicy } from "@/lib/clipboard";
import {
  pageDescriptionClassName,
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

interface IdentifyMatch {
  wallet: string;
  account: string;
  chain: string;
  network: string;
  branch: string;
  address_index: number | null;
  derivation_path: string | null;
  match_source: string;
}

interface IdentifyResult {
  input: string;
  type: string;
  chain: string;
  status: string;
  classification: string;
  note: string;
  matches?: IdentifyMatch[];
  wallets?: string[];
  owned_inputs?: number | null;
  owned_outputs?: number | null;
  external_outputs?: number | null;
  match_source?: string;
  legs?: Array<{
    side: string;
    outpoint?: string | null;
    n?: number | null;
    owned: boolean;
    wallet: string;
    branch?: string;
  }>;
}

interface IdentifySummary {
  total: number;
  owned: number;
  external: number;
  unknown: number;
  invalid: number;
  /** Recognized formats the check does not handle yet (invoices, xpubs…). */
  unsupported?: number;
  wallets_scanned: number;
  scan_to_index: number;
  verified_on_chain: boolean;
}

interface IdentifyReport {
  results: IdentifyResult[];
  summary: IdentifySummary;
  warnings: string[];
  context?: { workspace: string | null; profile: string | null };
}

type IdentifyArgs = { text?: string; csv_text?: string; backend?: string };

/** Streamed by the daemon after each on-chain lookup. */
interface VerifyProgress {
  checked: number;
  total: number;
}

interface BackendOption {
  name: string;
  display_name?: string;
  kind: string;
  chain?: string;
  network?: string;
  is_default?: boolean;
}

type Status = "owned" | "external" | "unknown" | "invalid" | "unsupported";
type StatusFilter = "all" | Status;

// Stable status id → tone + translation key. The label is resolved from the
// `review` namespace at render so test ids/lookups stay decoupled.
const STATUS_TONE: Record<Status, { dot: string; labelKey: string }> = {
  owned: { dot: "bg-emerald-500", labelKey: "reconcile.status.owned" },
  external: { dot: "bg-muted-foreground/60", labelKey: "reconcile.status.external" },
  unknown: { dot: "bg-amber-500", labelKey: "reconcile.status.unknown" },
  invalid: { dot: "bg-destructive", labelKey: "reconcile.status.invalid" },
  unsupported: { dot: "bg-sky-500", labelKey: "reconcile.status.unsupported" },
};

// Entry types the daemon names; anything newer shows its raw id until it gets
// a label here.
const TYPE_LABEL_KEY: Record<string, string> = {
  address: "reconcile.type.address",
  txid: "reconcile.type.txid",
  lightning_invoice: "reconcile.type.lightningInvoice",
  lightning_offer: "reconcile.type.lightningOffer",
  lnurl: "reconcile.type.lnurl",
  lightning_address: "reconcile.type.lightningAddress",
  silent_payment_address: "reconcile.type.silentPaymentAddress",
  outpoint: "reconcile.type.outpoint",
  extended_public_key: "reconcile.type.extendedPublicKey",
  private_key: "reconcile.type.privateKey",
};

function statusOf(result: IdentifyResult): Status {
  return result.status in STATUS_TONE ? (result.status as Status) : "unknown";
}

const CLASSIFICATION_LABEL_KEY: Record<string, string> = {
  owned_address: "reconcile.classification.ownedAddress",
  external_address: "reconcile.classification.externalAddress",
  self_transfer: "reconcile.classification.selfTransfer",
  outbound_payment: "reconcile.classification.outboundPayment",
  inbound_receipt: "reconcile.classification.inboundReceipt",
  touches_wallet: "reconcile.classification.touchesWallet",
  external: "reconcile.classification.external",
  unknown: "reconcile.classification.unknown",
  undetermined: "reconcile.classification.undetermined",
  invalid: "reconcile.classification.invalid",
  unsupported: "reconcile.classification.unsupported",
};

function ownerLabel(result: IdentifyResult): string {
  if (result.matches && result.matches.length > 0) {
    return Array.from(new Set(result.matches.map((m) => m.wallet).filter(Boolean))).join(", ");
  }
  if (result.wallets && result.wallets.length > 0) {
    return result.wallets.filter(Boolean).join(", ");
  }
  return "";
}

function branchLabel(result: IdentifyResult): string {
  const primary = result.matches?.[0];
  if (!primary) return "";
  const where = primary.branch || "address";
  return primary.address_index != null ? `${where} #${primary.address_index}` : where;
}

function csvCell(value: string): string {
  if (/[",\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

function resultsToCsv(results: IdentifyResult[]): string {
  const header = [
    "input",
    "type",
    "chain",
    "status",
    "classification",
    "wallet",
    "branch",
    "note",
  ];
  const lines = [header.join(",")];
  for (const result of results) {
    lines.push(
      [
        result.input,
        result.type,
        result.chain,
        result.status,
        result.classification,
        ownerLabel(result),
        branchLabel(result),
        result.note,
      ]
        .map((value) => csvCell(String(value ?? "")))
        .join(","),
    );
  }
  return lines.join("\n");
}

/**
 * How many distinct entries the field holds, split the way the daemon splits
 * pasted text. A hint that the paste was read, not a validation: the daemon
 * still decides what is an address, a txid, or invalid.
 */
function countEntries(text: string): number {
  return new Set(text.split(/[\s,;|\t]+/).filter(Boolean)).size;
}

function StatusLabel({ status }: { status: Status }) {
  const { t } = useTranslation("review");
  return (
    <span className="inline-flex items-center gap-1.5 text-sm whitespace-nowrap">
      <span
        className={cn("size-2 shrink-0 rounded-full", STATUS_TONE[status].dot)}
        aria-hidden="true"
      />
      {/* dynamic key */}
      {t(STATUS_TONE[status].labelKey as never)}
    </span>
  );
}

type Leg = NonNullable<IdentifyResult["legs"]>[number];

function truncateMiddle(value: string, head = 10, tail = 6): string {
  return value.length > head + tail + 1
    ? `${value.slice(0, head)}…${value.slice(-tail)}`
    : value;
}

function LegRow({ leg, hideSensitive }: { leg: Leg; hideSensitive: boolean }) {
  const { t } = useTranslation("review");
  const label =
    leg.side === "input"
      ? leg.outpoint
        ? truncateMiddle(leg.outpoint)
        : t("reconcile.legs.input")
      : `#${leg.n ?? "?"}`;
  return (
    <li className="flex items-center justify-between gap-3 py-1">
      <span
        className={cn("font-mono text-xs", hiddenSensitiveClassName(hideSensitive))}
      >
        {label}
      </span>
      {leg.owned ? (
        <span className="truncate text-xs font-medium text-emerald-700 dark:text-emerald-400">
          {leg.wallet}
          {leg.branch ? ` · ${leg.branch}` : ""}
        </span>
      ) : (
        <span className="text-xs text-muted-foreground">
          {t("reconcile.legs.external")}
        </span>
      )}
    </li>
  );
}

function LegColumn({
  title,
  legs,
  hideSensitive,
}: {
  title: string;
  legs: Leg[];
  hideSensitive: boolean;
}) {
  return (
    <div className="kb-surface-inset min-w-0 flex-1 px-3 py-2">
      <p className="text-xs font-medium text-muted-foreground">
        {title} <span className="tabular-nums">{legs.length}</span>
      </p>
      {legs.length === 0 ? (
        <p className="py-1 text-xs text-muted-foreground">—</p>
      ) : (
        <ul className="divide-y">
          {legs.map((leg, index) => (
            <LegRow key={index} leg={leg} hideSensitive={hideSensitive} />
          ))}
        </ul>
      )}
    </div>
  );
}

function LegsBreakdown({
  legs,
  hideSensitive,
}: {
  legs: Leg[];
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("review");
  return (
    <div className="flex flex-col gap-2 sm:flex-row sm:items-start">
      <LegColumn
        title={t("reconcile.legs.inputs")}
        legs={legs.filter((leg) => leg.side === "input")}
        hideSensitive={hideSensitive}
      />
      <ArrowRight
        className="mx-auto size-4 shrink-0 rotate-90 self-center text-muted-foreground sm:rotate-0"
        aria-hidden="true"
      />
      <LegColumn
        title={t("reconcile.legs.outputs")}
        legs={legs.filter((leg) => leg.side === "output")}
        hideSensitive={hideSensitive}
      />
    </div>
  );
}

function EmptyResults() {
  const { t } = useTranslation("review");
  const legend: Array<[Status, string]> = [
    ["owned", t("reconcile.empty.owned")],
    ["external", t("reconcile.empty.external")],
    ["unknown", t("reconcile.empty.unknown")],
  ];
  return (
    <div className="kb-surface p-(--kb-card-padding)">
      <h2 className="text-sm font-semibold">{t("reconcile.empty.title")}</h2>
      <dl className="mt-3 grid gap-2.5">
        {legend.map(([status, body]) => (
          <div key={status} className="grid gap-0.5 sm:grid-cols-[7rem_minmax(0,1fr)] sm:gap-3">
            <dt>
              <StatusLabel status={status} />
            </dt>
            <dd className="text-sm text-muted-foreground">{body}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export function Reconcile() {
  const { t } = useTranslation(["review", "common"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [input, setInput] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState<StatusFilter>("all");
  const [query, setQuery] = React.useState("");
  const [copied, setCopied] = React.useState(false);
  // The displayed report comes from either the cache-only check or the on-chain
  // verify, whichever ran most recently.
  const [report, setReport] = React.useState<IdentifyReport | null>(null);
  // What the shown report was checked with. "Verify on chain" re-runs exactly
  // this, not whatever the field holds now, so editing the list after a check
  // cannot change what gets sent to the backend.
  const [checkedArgs, setCheckedArgs] = React.useState<IdentifyArgs | null>(null);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [expanded, setExpanded] = React.useState<Set<string>>(() => new Set());
  // Smart CSV import: the file's content travels as csv_text and is harvested
  // daemon-side, so it works in every runtime (Tauri webview, bridge, browser)
  // with no daemon filesystem read.
  const [csvText, setCsvText] = React.useState<string | null>(null);
  const [csvName, setCsvName] = React.useState<string | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const check = useDaemonMutation<IdentifyReport>("ui.wallets.identify");
  // Verify streams: the daemon looks transactions up one by one on a worker,
  // reporting progress, and Stop names this run's request id to cancel it.
  const [verifyProgress, setVerifyProgress] = React.useState<VerifyProgress | null>(null);
  const verifyRequestId = React.useRef<string | null>(null);
  const verify = useDaemonStreamMutation<IdentifyReport, VerifyProgress>(
    "ui.wallets.identify_onchain",
    {
      onProgress: setVerifyProgress,
      requestId: () => {
        const id = makeDaemonRequestId();
        verifyRequestId.current = id;
        return id;
      },
    },
  );
  const cancelVerify = useDaemonMutation("ui.wallets.identify_onchain.cancel");
  const onStopVerify = () => {
    const target = verifyRequestId.current;
    if (target) cancelVerify.mutate({ target_request_id: target });
  };
  const busy = check.isPending || verify.isPending;

  const results = React.useMemo(() => report?.results ?? [], [report]);
  const summary = report?.summary;
  const unknownCount = summary?.unknown ?? 0;
  const txidsMissingLegs = React.useMemo(
    () =>
      results.filter(
        (result) =>
          result.type === "txid" &&
          result.status !== "invalid" &&
          (result.legs?.length ?? 0) === 0,
      ).length,
    [results],
  );
  const verifyCount = Math.max(unknownCount, txidsMissingLegs);
  // Which backend "Verify on chain" asks. A config read, no network; only
  // Esplora and Electrum can answer a transaction lookup. Defaults to the
  // book's default backend, so a Liquid list can be pointed at a Liquid one.
  const backendsQuery = useDaemon<{ backends: BackendOption[] }>(
    "ui.backends.options",
    undefined,
    { enabled: verifyCount > 0 },
  );
  const verifyBackends = (backendsQuery.data?.data?.backends ?? []).filter(
    (backend) => /esplora|electrum/i.test(backend.kind),
  );
  const [verifyBackendName, setVerifyBackendName] = React.useState<string | null>(null);
  const verifyBackend =
    verifyBackends.find((backend) => backend.name === verifyBackendName) ??
    verifyBackends.find((backend) => backend.is_default) ??
    verifyBackends[0];

  const trimmed = input.trim();
  const hasInput = trimmed.length > 0 || !!csvText;
  const entryCount = React.useMemo(() => countEntries(input), [input]);

  const currentArgs = (csvOverride?: string | null): IdentifyArgs | null => {
    const csv = csvOverride !== undefined ? csvOverride : csvText;
    if (!trimmed && !csv) return null;
    const args: IdentifyArgs = {};
    if (trimmed) args.text = input;
    if (csv) args.csv_text = csv;
    return args;
  };

  const runMutation = async (
    mutation: {
      mutateAsync: (args: IdentifyArgs) => Promise<{ data?: IdentifyReport | null }>;
    },
    failureLabel: string,
    args: IdentifyArgs | null,
  ) => {
    if (!args) return;
    setErrorMessage(null);
    setStatusFilter("all");
    setQuery("");
    setExpanded(new Set());
    try {
      const envelope = await mutation.mutateAsync(args);
      setReport(envelope.data ?? null);
      setCheckedArgs(args);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : failureLabel);
    }
  };

  const onCheck = () => {
    if (!hasInput || busy) return;
    void runMutation(check, t("reconcile.checkFailed"), currentArgs());
  };
  const onVerify = async () => {
    setVerifyProgress(null);
    await runMutation(
      verify,
      t("reconcile.verifyFailed"),
      checkedArgs && {
        ...checkedArgs,
        ...(verifyBackend ? { backend: verifyBackend.name } : {}),
      },
    );
    setVerifyProgress(null);
    verifyRequestId.current = null;
  };
  const listChanged =
    checkedArgs !== null &&
    ((checkedArgs.text ?? "") !== (trimmed ? input : "") ||
      (checkedArgs.csv_text ?? null) !== csvText);

  const onImportCsv = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = ""; // allow re-importing the same file
    if (!file) return;
    try {
      const content = await file.text();
      if (!content.trim()) {
        setErrorMessage(t("reconcile.fileEmpty"));
        return;
      }
      setCsvText(content);
      setCsvName(file.name);
      await runMutation(check, t("reconcile.csvImportFailed"), currentArgs(content));
    } catch {
      setErrorMessage(t("reconcile.fileReadFailed"));
    }
  };

  const clearInput = () => {
    setInput("");
    setCsvText(null);
    setCsvName(null);
  };

  const toggleExpand = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const filtered = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    return results.filter((result) => {
      if (statusFilter !== "all" && statusOf(result) !== statusFilter) {
        return false;
      }
      if (!needle) return true;
      return [result.input, ownerLabel(result), branchLabel(result), result.classification]
        .join(" ")
        .toLowerCase()
        .includes(needle);
    });
  }, [results, statusFilter, query]);

  const onCopyCsv = async () => {
    if (results.length === 0) return;
    try {
      await copyTextWithPolicy(resultsToCsv(results));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard access is best-effort in browser preview.
    }
  };

  const filters: Array<{ id: StatusFilter; label: string; count: number }> =
    summary
      ? [
          { id: "all", label: t("reconcile.filterAll"), count: summary.total },
          { id: "owned", label: t("reconcile.status.owned"), count: summary.owned },
          { id: "external", label: t("reconcile.status.external"), count: summary.external },
          { id: "unknown", label: t("reconcile.status.unknown"), count: summary.unknown },
          ...(summary.invalid > 0
            ? [{ id: "invalid" as const, label: t("reconcile.status.invalid"), count: summary.invalid }]
            : []),
          ...((summary.unsupported ?? 0) > 0
            ? [{ id: "unsupported" as const, label: t("reconcile.status.unsupported"), count: summary.unsupported ?? 0 }]
            : []),
        ]
      : [];

  return (
    <div className={screenShellClassName}>
      <div className={pageHeaderClassName}>
        <p className={pageDescriptionClassName}>{t("reconcile.description")}</p>
        {summary ? (
          <div className={pageHeaderActionsClassName}>
            {verifyCount > 0 ? (
              <div className="flex items-center">
                <Button
                  type="button"
                  variant="outline"
                  className={cn(
                    pageHeaderActionClassName,
                    verifyBackends.length > 1 && "rounded-r-none",
                  )}
                  title={t("reconcile.verifyHint", {
                    backend: verifyBackend?.display_name || verifyBackend?.name || "",
                  })}
                  onClick={onVerify}
                  disabled={busy}
                >
                  <Globe className="size-4" aria-hidden="true" />
                  {verify.isPending
                    ? verifyProgress && verifyProgress.total > 0
                      ? t("reconcile.verifyingProgress", {
                          checked: verifyProgress.checked,
                          total: verifyProgress.total,
                        })
                      : t("reconcile.verifying")
                    : t("reconcile.verifyOnChain", { count: verifyCount })}
                </Button>
                {verifyBackends.length > 1 ? (
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="outline"
                        className="h-8 rounded-l-none border-l-0 px-2"
                        aria-label={t("reconcile.verifyBackend")}
                        disabled={busy}
                      >
                        <ChevronDown className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-64">
                      <DropdownMenuLabel className="text-xs font-medium text-muted-foreground">
                        {t("reconcile.verifyBackend")}
                      </DropdownMenuLabel>
                      <DropdownMenuRadioGroup
                        value={verifyBackend?.name}
                        onValueChange={setVerifyBackendName}
                      >
                        {verifyBackends.map((backend) => (
                          <DropdownMenuRadioItem key={backend.name} value={backend.name}>
                            <span className="min-w-0 flex-1 truncate">
                              {backend.display_name || backend.name}
                            </span>
                            <span className="text-xs text-muted-foreground">
                              {[backend.chain, backend.network].filter(Boolean).join(" · ")}
                            </span>
                          </DropdownMenuRadioItem>
                        ))}
                      </DropdownMenuRadioGroup>
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : null}
              </div>
            ) : null}
            {verify.isPending ? (
              <Button
                type="button"
                variant="ghost"
                className={pageHeaderActionClassName}
                onClick={onStopVerify}
                disabled={cancelVerify.isPending}
              >
                <Square className="size-3.5" aria-hidden="true" />
                {t("reconcile.stopVerify")}
              </Button>
            ) : null}
            <Button
              type="button"
              variant="outline"
              className={pageHeaderActionClassName}
              onClick={onCopyCsv}
              disabled={results.length === 0}
            >
              <ClipboardCopy className="size-4" aria-hidden="true" />
              {copied ? t("reconcile.copied") : t("reconcile.copyCsv")}
            </Button>
          </div>
        ) : null}
      </div>

      {/* 33rem keeps a 64-character txid on one line of the monospace field. */}
      <div className="grid items-start gap-(--kb-page-gap) xl:grid-cols-[minmax(0,33rem)_minmax(0,1fr)]">
        <section
          aria-label={t("reconcile.inputTitle")}
          className="kb-surface flex flex-col overflow-hidden focus-within:border-ring/60 xl:sticky xl:top-(--kb-page-gutter)"
        >
          <div className="flex items-center justify-between gap-2 border-b px-4 py-2.5">
            <h2 className="text-sm font-semibold">{t("reconcile.inputTitle")}</h2>
            {entryCount > 0 ? (
              <span className="text-xs text-muted-foreground tabular-nums">
                {t("reconcile.entryCount", { count: entryCount })}
              </span>
            ) : null}
          </div>
          <Textarea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                event.preventDefault();
                onCheck();
              }
            }}
            placeholder={t("reconcile.placeholder")}
            spellCheck={false}
            aria-label={t("reconcile.inputTitle")}
            className={cn(
              "min-h-72 max-h-[65vh] resize-none overflow-y-auto rounded-none border-0 bg-transparent px-4 py-3 font-mono text-xs shadow-none focus-visible:ring-0 md:text-xs dark:bg-transparent",
              hiddenSensitiveClassName(hideSensitive),
            )}
          />
          {csvName ? (
            <div className="px-4 pb-3">
              <span className="inline-flex max-w-full items-center gap-2 rounded-md border bg-muted/40 px-2 py-1 text-xs">
                <FileSpreadsheet className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                <span className="truncate font-medium">{csvName}</span>
                <button
                  type="button"
                  onClick={() => {
                    setCsvText(null);
                    setCsvName(null);
                  }}
                  aria-label={t("reconcile.removeCsvAria")}
                  className="rounded-sm text-muted-foreground hover:text-foreground"
                >
                  <X className="size-3.5" aria-hidden="true" />
                </button>
              </span>
            </div>
          ) : null}
          <input
            ref={fileInputRef}
            type="file"
            accept=".csv,.tsv,.txt,text/csv,text/plain"
            className="hidden"
            onChange={onImportCsv}
          />
          <div className="flex items-center gap-2 border-t bg-muted/20 px-3 py-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => fileInputRef.current?.click()}
              disabled={busy}
            >
              <FileSpreadsheet className="size-4" aria-hidden="true" />
              {t("reconcile.importCsv")}
            </Button>
            {hasInput ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={clearInput}
                disabled={busy}
              >
                {t("reconcile.clear")}
              </Button>
            ) : null}
            <Button
              type="button"
              size="sm"
              className="ml-auto"
              onClick={onCheck}
              disabled={!hasInput || busy}
            >
              <Search className="size-4" aria-hidden="true" />
              {check.isPending ? t("reconcile.checking") : t("reconcile.checkOwnership")}
              <Kbd className="bg-primary-foreground/15 text-primary-foreground/80">
                {formatShortcut(["mod", "enter"])}
              </Kbd>
            </Button>
          </div>
          <p className="flex items-start gap-1.5 border-t px-4 py-2.5 text-xs text-muted-foreground">
            <ShieldCheck
              className="mt-px size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400"
              aria-hidden="true"
            />
            {t("reconcile.onDeviceNote")}
          </p>
        </section>

        <div className="min-w-0 space-y-(--kb-page-gap)">
          {errorMessage ? (
            <div
              role="alert"
              className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
            >
              {errorMessage}
            </div>
          ) : null}

          {!summary ? (
            <EmptyResults />
          ) : (
            <section
              aria-label={t("reconcile.resultsTitle")}
              className="kb-surface overflow-hidden"
            >
              <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2.5">
                <div
                  role="group"
                  aria-label={t("common:actions.filter")}
                  className="inline-flex flex-wrap items-center gap-0.5 rounded-lg bg-muted p-0.5"
                >
                  {filters.map((filter) => (
                    <button
                      key={filter.id}
                      type="button"
                      aria-pressed={statusFilter === filter.id}
                      onClick={() => setStatusFilter(filter.id)}
                      className={cn(
                        "inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-sm font-medium transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                        statusFilter === filter.id
                          ? "bg-background text-foreground shadow-xs"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      {filter.id !== "all" ? (
                        <span
                          className={cn("size-1.5 rounded-full", STATUS_TONE[filter.id].dot)}
                          aria-hidden="true"
                        />
                      ) : null}
                      {filter.label}
                      <span className="text-xs tabular-nums text-muted-foreground">
                        {filter.count}
                      </span>
                    </button>
                  ))}
                </div>
                <div className="relative ml-auto min-w-40 flex-1 sm:max-w-56">
                  <Search
                    className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder={t("reconcile.filterPlaceholder")}
                    aria-label={t("reconcile.filterPlaceholder")}
                    className="h-8 w-full rounded-md border border-input bg-transparent pr-2 pl-8 text-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30"
                  />
                </div>
              </div>

              <Table>
                <TableHeader>
                  <TableRow className="hover:bg-transparent">
                    <TableHead className="w-8 pr-0" aria-hidden="true" />
                    <TableHead>{t("reconcile.table.input")}</TableHead>
                    <TableHead className="w-28">{t("common:field.status")}</TableHead>
                    <TableHead>{t("reconcile.table.match")}</TableHead>
                    <TableHead>{t("reconcile.table.classification")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filtered.length === 0 ? (
                    <TableRow className="hover:bg-transparent">
                      <TableCell
                        colSpan={5}
                        className="py-8 text-center text-sm text-muted-foreground"
                      >
                        {t("reconcile.table.noResults")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    filtered.map((result, index) => {
                      const owner = ownerLabel(result);
                      const branch = branchLabel(result);
                      const rowKey = `${result.input}-${index}`;
                      const legs = result.legs ?? [];
                      const hasLegs = legs.length > 0;
                      const isOpen = expanded.has(rowKey);
                      return (
                        <React.Fragment key={rowKey}>
                          <TableRow
                            data-state={isOpen ? "selected" : undefined}
                            className={cn(hasLegs && "cursor-pointer")}
                            onClick={(event) => {
                              if (!hasLegs) return;
                              if ((event.target as HTMLElement).closest("button, a")) return;
                              toggleExpand(rowKey);
                            }}
                          >
                            <TableCell className="w-8 pr-0 align-top">
                              {hasLegs ? (
                                <button
                                  type="button"
                                  onClick={() => toggleExpand(rowKey)}
                                  aria-label={isOpen ? t("reconcile.hideLegsAria") : t("reconcile.showLegsAria")}
                                  aria-expanded={isOpen}
                                  className="mt-0.5 rounded-md p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                                >
                                  {isOpen ? (
                                    <ChevronDown className="size-3.5" aria-hidden="true" />
                                  ) : (
                                    <ChevronRight className="size-3.5" aria-hidden="true" />
                                  )}
                                </button>
                              ) : null}
                            </TableCell>
                            <TableCell className="max-w-[22rem] align-top">
                              <div className="group/entry flex min-w-0 items-center gap-1">
                                <span
                                  className={cn(
                                    "truncate font-mono text-xs",
                                    hiddenSensitiveClassName(hideSensitive),
                                  )}
                                  title={result.input}
                                >
                                  {result.input}
                                </span>
                                <span className="opacity-0 transition-opacity group-hover/entry:opacity-100 focus-within:opacity-100">
                                  <CopyButton value={result.input} ariaLabel={t("reconcile.copyInputAria")} />
                                </span>
                              </div>
                              <span className="text-xs text-muted-foreground">
                                {TYPE_LABEL_KEY[result.type]
                                  ? // dynamic key
                                    t(TYPE_LABEL_KEY[result.type] as never)
                                  : result.type}
                                {result.chain ? ` · ${result.chain}` : ""}
                              </span>
                            </TableCell>
                            <TableCell className="align-top">
                              <StatusLabel status={statusOf(result)} />
                            </TableCell>
                            <TableCell className="align-top text-sm">
                              {owner ? (
                                <>
                                  <span className="font-medium">{owner}</span>
                                  {branch ? (
                                    <span className="block text-xs text-muted-foreground">{branch}</span>
                                  ) : null}
                                </>
                              ) : (
                                <span className="text-muted-foreground">—</span>
                              )}
                            </TableCell>
                            <TableCell className="align-top text-sm whitespace-normal">
                              {CLASSIFICATION_LABEL_KEY[result.classification]
                                ? // dynamic key
                                  t(CLASSIFICATION_LABEL_KEY[result.classification] as never)
                                : result.classification}
                              {result.note ? (
                                <p className="text-xs text-muted-foreground">{result.note}</p>
                              ) : null}
                            </TableCell>
                          </TableRow>
                          {hasLegs && isOpen ? (
                            <TableRow className="hover:bg-transparent">
                              <TableCell />
                              <TableCell colSpan={4} className="pt-0 pb-3 whitespace-normal">
                                <LegsBreakdown legs={legs} hideSensitive={hideSensitive} />
                              </TableCell>
                            </TableRow>
                          ) : null}
                        </React.Fragment>
                      );
                    })
                  )}
                </TableBody>
              </Table>

              <div className="space-y-2 border-t px-4 py-2.5">
                {listChanged ? (
                  <p className="text-xs font-medium text-amber-700 dark:text-amber-300">
                    {t("reconcile.listChanged")}
                  </p>
                ) : null}
                <p className="text-xs text-muted-foreground">
                  {t("reconcile.scanned", {
                    count: summary.wallets_scanned,
                    index: summary.scan_to_index,
                    verified: summary.verified_on_chain
                      ? t("reconcile.verifiedSuffix")
                      : "",
                  })}
                </p>
                {report?.warnings && report.warnings.length > 0 ? (
                  <ul className="space-y-1 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-200">
                    {report.warnings.map((warning, index) => (
                      <li key={index} className="flex items-start gap-1.5">
                        <TriangleAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                        {warning}
                      </li>
                    ))}
                  </ul>
                ) : null}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}

export default Reconcile;
