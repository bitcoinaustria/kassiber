/**
 * Reconcile — address / transaction-id ownership lookup.
 *
 * Paste a pile of addresses or txids (Bitcoin or Liquid, mixed) and see which
 * belong to a wallet in the active profile — naming the wallet and whether it
 * is a receive or change address — and which are external. The reconciliation
 * workflow for telling apart historic payments from transfers between your own
 * wallets. Matching runs locally against synced inventory and offline
 * descriptor derivation; nothing leaves the device. Deeper / on-chain
 * verification is available from the `kassiber wallets identify` CLI.
 */
import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  ClipboardCopy,
  FileSpreadsheet,
  Fingerprint,
  Globe,
  Search,
  ShieldCheck,
  X,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import { useDaemonMutation } from "@/daemon/client";
import { copyTextWithPolicy } from "@/lib/clipboard";
import { screenShellClassName } from "@/lib/screen-layout";
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
  // Per-leg detail is carried on txid rows by the real payload; the screen does
  // not render it yet, but the field is typed so the shape stays in lockstep.
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

type StatusFilter = "all" | "owned" | "external" | "unknown" | "invalid";

// Stable status id → badge variant + translation key. The label is resolved
// from the `review` namespace at render so test ids/lookups stay decoupled.
const STATUS_BADGE: Record<
  string,
  {
    variant: "default" | "secondary" | "outline" | "destructive";
    labelKey: string;
  }
> = {
  owned: { variant: "default", labelKey: "reconcile.status.owned" },
  external: { variant: "secondary", labelKey: "reconcile.status.external" },
  unknown: { variant: "outline", labelKey: "reconcile.status.unknown" },
  invalid: { variant: "destructive", labelKey: "reconcile.status.invalid" },
};

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

function MetricTile({
  label,
  value,
  active,
  onClick,
}: {
  label: string;
  value: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex min-w-24 flex-1 flex-col rounded-lg border px-3 py-2 text-left transition-colors ${
        active
          ? "border-primary bg-primary/5"
          : "border-border bg-card hover:bg-accent"
      }`}
    >
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-xl font-semibold tabular-nums">{value}</span>
    </button>
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
    <div className="flex items-center justify-between gap-2 py-0.5">
      <span
        className={`font-mono text-[11px] ${hiddenSensitiveClassName(hideSensitive)}`}
      >
        {label}
      </span>
      {leg.owned ? (
        <span className="text-[11px] text-emerald-600 dark:text-emerald-400">
          {leg.wallet}
          {leg.branch ? ` · ${leg.branch}` : ""}
        </span>
      ) : (
        <span className="text-[11px] text-muted-foreground">{t("reconcile.legs.external")}</span>
      )}
    </div>
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
    <div>
      <p className="mb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {title} ({legs.length})
      </p>
      {legs.length === 0 ? (
        <p className="text-[11px] text-muted-foreground">—</p>
      ) : (
        legs.map((leg, index) => (
          <LegRow key={index} leg={leg} hideSensitive={hideSensitive} />
        ))
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
    <div className="grid gap-4 rounded-md bg-muted/40 p-3 sm:grid-cols-2">
      <LegColumn
        title={t("reconcile.legs.inputs")}
        legs={legs.filter((leg) => leg.side === "input")}
        hideSensitive={hideSensitive}
      />
      <LegColumn
        title={t("reconcile.legs.outputs")}
        legs={legs.filter((leg) => leg.side === "output")}
        hideSensitive={hideSensitive}
      />
    </div>
  );
}

export function Reconcile() {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [input, setInput] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState<StatusFilter>("all");
  const [copied, setCopied] = React.useState(false);
  // The displayed report comes from either the cache-only check or the on-chain
  // verify, whichever ran most recently.
  const [report, setReport] = React.useState<IdentifyReport | null>(null);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [expanded, setExpanded] = React.useState<Set<string>>(() => new Set());
  // Smart CSV import: the file's content travels as csv_text and is harvested
  // daemon-side, so it works in every runtime (Tauri webview, bridge, browser)
  // with no daemon filesystem read.
  const [csvText, setCsvText] = React.useState<string | null>(null);
  const [csvName, setCsvName] = React.useState<string | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const check = useDaemonMutation<IdentifyReport>("ui.wallets.identify");
  const verify = useDaemonMutation<IdentifyReport>("ui.wallets.identify_onchain");

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

  const trimmed = input.trim();
  const hasInput = trimmed.length > 0 || !!csvText;

  const runMutation = async (
    mutation: typeof check,
    failureLabel: string,
    csvOverride?: string | null,
  ) => {
    const csv = csvOverride !== undefined ? csvOverride : csvText;
    if (!trimmed && !csv) return;
    setErrorMessage(null);
    setStatusFilter("all");
    setExpanded(new Set());
    try {
      const args: Record<string, unknown> = {};
      if (trimmed) args.text = input;
      if (csv) args.csv_text = csv;
      const envelope = await mutation.mutateAsync(args);
      setReport(envelope.data ?? null);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : failureLabel);
    }
  };

  const onCheck = () => runMutation(check, t("reconcile.checkFailed"));
  const onVerify = () => runMutation(verify, t("reconcile.verifyFailed"));

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
      await runMutation(check, t("reconcile.csvImportFailed"), content);
    } catch {
      setErrorMessage(t("reconcile.fileReadFailed"));
    }
  };

  const clearCsv = () => {
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

  const filtered = React.useMemo(
    () =>
      statusFilter === "all"
        ? results
        : results.filter((r) => r.status === statusFilter),
    [results, statusFilter],
  );

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

  return (
    <div className={screenShellClassName}>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Fingerprint className="size-5 text-primary" />
            {t("reconcile.title")}
          </CardTitle>
          <CardDescription>
            {t("reconcile.description")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Textarea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            placeholder={t("reconcile.placeholder")}
            rows={6}
            spellCheck={false}
            className={`font-mono text-xs ${hiddenSensitiveClassName(hideSensitive)}`}
          />
          {csvName ? (
            <div className="flex w-fit items-center gap-2 rounded-md border border-border bg-muted/40 px-2 py-1 text-xs">
              <FileSpreadsheet className="size-3.5 text-muted-foreground" />
              <span className="font-medium">{csvName}</span>
              <button
                type="button"
                onClick={clearCsv}
                aria-label={t("reconcile.removeCsvAria")}
                className="text-muted-foreground hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            </div>
          ) : null}
          <input
            ref={fileInputRef}
            type="file"
            accept=".csv,.tsv,.txt,text/csv,text/plain"
            className="hidden"
            onChange={onImportCsv}
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <ShieldCheck className="size-3.5 text-emerald-600 dark:text-emerald-400" />
              {t("reconcile.onDeviceNote")}
            </p>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => fileInputRef.current?.click()}
                disabled={check.isPending || verify.isPending}
              >
                <FileSpreadsheet className="size-4" />
                {t("reconcile.importCsv")}
              </Button>
              <Button
                type="button"
                onClick={onCheck}
                disabled={!hasInput || check.isPending || verify.isPending}
              >
                <Search className="size-4" />
                {check.isPending ? t("reconcile.checking") : t("reconcile.checkOwnership")}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {errorMessage ? (
        <Card>
          <CardContent className="py-4 text-sm text-destructive">
            {errorMessage}
          </CardContent>
        </Card>
      ) : null}

      {summary ? (
        <>
          <div className="flex flex-wrap gap-2">
            <MetricTile
              label={t("reconcile.metric.checked")}
              value={summary.total}
              active={statusFilter === "all"}
              onClick={() => setStatusFilter("all")}
            />
            <MetricTile
              label={t("reconcile.metric.owned")}
              value={summary.owned}
              active={statusFilter === "owned"}
              onClick={() => setStatusFilter("owned")}
            />
            <MetricTile
              label={t("reconcile.metric.external")}
              value={summary.external}
              active={statusFilter === "external"}
              onClick={() => setStatusFilter("external")}
            />
            <MetricTile
              label={t("reconcile.metric.unknown")}
              value={summary.unknown}
              active={statusFilter === "unknown"}
              onClick={() => setStatusFilter("unknown")}
            />
            {summary.invalid > 0 ? (
              <MetricTile
                label={t("reconcile.metric.invalid")}
                value={summary.invalid}
                active={statusFilter === "invalid"}
                onClick={() => setStatusFilter("invalid")}
              />
            ) : null}
          </div>

          <Card>
            <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
              <CardDescription>
                {t("reconcile.scanned", {
                  count: summary.wallets_scanned,
                  index: summary.scan_to_index,
                  verified: summary.verified_on_chain
                    ? t("reconcile.verifiedSuffix")
                    : "",
                })}
              </CardDescription>
              <div className="flex items-center gap-2">
                {verifyCount > 0 ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={onVerify}
                    disabled={verify.isPending || check.isPending}
                  >
                    <Globe className="size-4" />
                    {verify.isPending
                      ? t("reconcile.verifying")
                      : t("reconcile.verifyOnChain", { count: verifyCount })}
                  </Button>
                ) : null}
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={onCopyCsv}
                  disabled={results.length === 0}
                >
                  <ClipboardCopy className="size-4" />
                  {copied ? t("reconcile.copied") : t("reconcile.copyCsv")}
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("reconcile.table.input")}</TableHead>
                    <TableHead>{t("common:field.status")}</TableHead>
                    <TableHead>{t("reconcile.table.wallet")}</TableHead>
                    <TableHead>{t("reconcile.table.branch")}</TableHead>
                    <TableHead>{t("reconcile.table.classification")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filtered.length === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={5}
                        className="py-6 text-center text-sm text-muted-foreground"
                      >
                        {t("reconcile.table.noResults")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    filtered.map((result, index) => {
                      const badge =
                        STATUS_BADGE[result.status] ?? STATUS_BADGE.unknown;
                      const owner = ownerLabel(result);
                      const rowKey = `${result.input}-${index}`;
                      const legs = result.legs ?? [];
                      const hasLegs = legs.length > 0;
                      const isOpen = expanded.has(rowKey);
                      return (
                        <React.Fragment key={rowKey}>
                          <TableRow>
                            <TableCell className="max-w-[22rem]">
                              <div className="flex items-center gap-1.5">
                                {hasLegs ? (
                                  <button
                                    type="button"
                                    onClick={() => toggleExpand(rowKey)}
                                    aria-label={isOpen ? t("reconcile.hideLegsAria") : t("reconcile.showLegsAria")}
                                    aria-expanded={isOpen}
                                    className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                                  >
                                    {isOpen ? (
                                      <ChevronDown className="size-3.5" />
                                    ) : (
                                      <ChevronRight className="size-3.5" />
                                    )}
                                  </button>
                                ) : (
                                  <span className="w-[1.125rem]" aria-hidden />
                                )}
                                <span
                                  className={`truncate font-mono text-xs ${hiddenSensitiveClassName(hideSensitive)}`}
                                  title={result.input}
                                >
                                  {result.input}
                                </span>
                                <CopyButton value={result.input} ariaLabel={t("reconcile.copyInputAria")} />
                              </div>
                              <span className="ml-[1.625rem] text-[11px] uppercase tracking-wide text-muted-foreground">
                                {result.type}
                                {result.chain ? ` · ${result.chain}` : ""}
                              </span>
                            </TableCell>
                            <TableCell>
                              <Badge variant={badge.variant}>{t(badge.labelKey)}</Badge>
                            </TableCell>
                            <TableCell className="text-sm">
                              {owner || (
                                <span className="text-muted-foreground">—</span>
                              )}
                            </TableCell>
                            <TableCell className="text-sm text-muted-foreground">
                              {branchLabel(result) || "—"}
                            </TableCell>
                            <TableCell className="text-sm">
                              <span>
                                {CLASSIFICATION_LABEL_KEY[result.classification]
                                  ? t(CLASSIFICATION_LABEL_KEY[result.classification])
                                  : result.classification}
                              </span>
                              {result.note ? (
                                <p className="text-xs text-muted-foreground">
                                  {result.note}
                                </p>
                              ) : null}
                            </TableCell>
                          </TableRow>
                          {hasLegs && isOpen ? (
                            <TableRow>
                              <TableCell colSpan={5} className="bg-muted/20 py-2">
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
            </CardContent>
          </Card>

          {report?.warnings && report.warnings.length > 0 ? (
            <Card>
              <CardContent className="space-y-1 py-3 text-xs text-muted-foreground">
                {report.warnings.map((warning, index) => (
                  <p key={index}>⚠ {warning}</p>
                ))}
              </CardContent>
            </Card>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

export default Reconcile;
