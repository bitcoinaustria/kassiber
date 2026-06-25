/**
 * Loans — Bitcoin-backed lending (custodial + non-custodial / multisig).
 *
 * Simple by default, granular on demand. The default flow is three plain
 * questions (am I borrowing or lending, which provider, who holds the
 * collateral); everything else lives behind an "Advanced" disclosure. The
 * collateral lock is a non-event for tax (the coins stay in the owned pool,
 * encumbered) — this screen never does tax math; the daemon classifies legs by
 * role. Status follows signal-not-reassurance: a healthy loan shows no badge,
 * only actionable items surface.
 */

import { useState } from "react";
import { Banknote, FileDown, HandCoins, Info, Loader2, Plus, ShieldAlert, Upload } from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import { useTranslation } from "react-i18next";

interface LoanLeg {
  id: string;
  role: string;
  transaction_id: string | null;
  amount: number | null;
}

interface Loan {
  id: string;
  role: string;
  platform: string | null;
  custody_type: string | null;
  rehypothecation: string;
  status: string;
  collateral_asset: string;
  legs?: LoanLeg[];
  advisory?: string[];
}

interface LoanAction {
  loan_id: string;
  platform?: string | null;
  action: string;
  detail: string;
}

interface LoanPreset {
  preset_id: string;
  label: string;
  custody_type: string | null;
}

interface LoansSnapshot {
  loans: Loan[];
  actions: LoanAction[];
  presets: LoanPreset[];
  enums: {
    roles: string[];
    custody_types: string[];
    statuses: string[];
    leg_roles: string[];
  };
}

const CUSTODY_SIMPLE = [
  "non_custodial_multisig",
  "non_custodial_presigned",
  "custodial_segregated",
];

export function Loans() {
  const { t } = useTranslation(["loans", "common"]);
  const snapshot = useDaemon<LoansSnapshot>("ui.loans.list");
  const createLoan = useDaemonMutation("ui.loans.create");
  const addLeg = useDaemonMutation("ui.loans.add_leg");
  const updateLoan = useDaemonMutation("ui.loans.update");
  const importLoan = useDaemonMutation("ui.loans.import");
  const exportLoans = useDaemonMutation<{ loans: unknown[] }>("ui.loans.export");

  const [creating, setCreating] = useState(false);
  const [role, setRole] = useState("borrower");
  const [preset, setPreset] = useState<string>("");
  const [custody, setCustody] = useState("non_custodial_multisig");
  const [lockTxid, setLockTxid] = useState<Record<string, string>>({});
  const [importing, setImporting] = useState(false);
  const [importFmt, setImportFmt] = useState("csv");
  const [importText, setImportText] = useState("");
  const [banner, setBanner] = useState<string | null>(null);

  const data = snapshot.data?.data;

  if (snapshot.isLoading || !data) {
    return <ScreenSkeleton />;
  }

  const refresh = () => snapshot.refetch();

  const submitCreate = () => {
    createLoan.mutate(
      { role, preset: preset || undefined, custody_type: custody },
      {
        onSuccess: () => {
          setCreating(false);
          setPreset("");
          refresh();
        },
      },
    );
  };

  const submitLock = (loanId: string) => {
    const txid = (lockTxid[loanId] ?? "").trim();
    if (!txid) return;
    addLeg.mutate(
      { loan_id: loanId, role: "collateral_lock", txid },
      {
        onSuccess: () => {
          setLockTxid((prev) => ({ ...prev, [loanId]: "" }));
          refresh();
        },
      },
    );
  };

  const markStatus = (loanId: string, status: string) => {
    updateLoan.mutate({ loan_id: loanId, status }, { onSuccess: refresh });
  };

  const submitImport = () => {
    if (!importText.trim()) return;
    importLoan.mutate(
      { format: importFmt, file_text: importText },
      {
        onSuccess: () => {
          setImporting(false);
          setImportText("");
          setBanner(t("import.done"));
          refresh();
        },
      },
    );
  };

  const runExport = () => {
    exportLoans.mutate(undefined, {
      onSuccess: (envelope) => {
        const count = envelope.data?.loans?.length ?? 0;
        setBanner(t("export.done", { count }));
      },
    });
  };

  return (
    <div className={screenShellClassName}>
      <header className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="flex items-start gap-3">
          <div className="grid size-10 shrink-0 place-items-center rounded-xl border bg-card text-primary">
            <HandCoins className="size-5" />
          </div>
          <div>
            <h1 className="text-xl font-semibold">{t("header.title")}</h1>
            <p className="max-w-2xl text-sm text-muted-foreground">
              {t("header.description")}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => setImporting((v) => !v)}>
            <Upload className="size-4" />
            {t("import.button")}
          </Button>
          <Button variant="outline" onClick={runExport} disabled={exportLoans.isPending}>
            <FileDown className="size-4" />
            {t("export.button")}
          </Button>
          <Button onClick={() => setCreating((v) => !v)}>
            <Plus className="size-4" />
            {t("actions.newLoan")}
          </Button>
        </div>
      </header>

      {banner && (
        <div className="flex items-center gap-2 rounded-lg border bg-card px-3 py-2 text-sm text-muted-foreground">
          <Info className="size-4 shrink-0" />
          {banner}
        </div>
      )}

      {/* Signal-not-reassurance: actionable items only. A healthy loan is silent. */}
      {data.actions.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <ShieldAlert className="size-4 text-amber-500" />
              {t("status.title")}
            </CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            {data.actions.map((item, idx) => (
              <div key={`${item.loan_id}-${item.action}-${idx}`} className="flex items-start gap-2 text-sm">
                <Badge variant="outline">{t(`status.actions.${item.action}`, item.action)}</Badge>
                <span className="text-muted-foreground">{item.detail}</span>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      {creating && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t("wizard.title")}</CardTitle>
            <CardDescription>{t("wizard.subtitle")}</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <div className="grid gap-4 sm:grid-cols-3">
              <div className="flex flex-col gap-1.5">
                <Label>{t("wizard.role")}</Label>
                <Select value={role} onValueChange={setRole}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {data.enums.roles.map((r) => (
                      <SelectItem key={r} value={r}>
                        {t(`roles.${r}`, r)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>{t("wizard.provider")}</Label>
                <Select
                  value={preset}
                  onValueChange={(value) => {
                    setPreset(value);
                    const match = data.presets.find((p) => p.preset_id === value);
                    if (match?.custody_type) setCustody(match.custody_type);
                  }}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("wizard.providerPlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    {data.presets.map((p) => (
                      <SelectItem key={p.preset_id} value={p.preset_id}>
                        {p.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-1.5">
                <Label>{t("wizard.custody")}</Label>
                <Select value={custody} onValueChange={setCustody}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CUSTODY_SIMPLE.map((c) => (
                      <SelectItem key={c} value={c}>
                        {t(`custody.${c}`, c)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="flex gap-2">
              <Button onClick={submitCreate} disabled={createLoan.isPending}>
                {createLoan.isPending && <Loader2 className="size-4 animate-spin" />}
                {t("wizard.create")}
              </Button>
              <Button variant="ghost" onClick={() => setCreating(false)}>
                {t("common:cancel", "Cancel")}
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {importing && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t("import.title")}</CardTitle>
            <CardDescription>{t("import.subtitle")}</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <div className="flex flex-col gap-1.5 sm:max-w-xs">
              <Label>{t("import.format")}</Label>
              <Select value={importFmt} onValueChange={setImportFmt}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {["csv", "unchained", "hodlhodl"].map((f) => (
                    <SelectItem key={f} value={f}>
                      {t(`import.formats.${f}`, f)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <Textarea
              value={importText}
              onChange={(e) => setImportText(e.target.value)}
              placeholder={t("import.placeholder")}
              rows={6}
              className="font-mono text-xs"
            />
            <div className="flex gap-2">
              <Button onClick={submitImport} disabled={importLoan.isPending || !importText.trim()}>
                {importLoan.isPending && <Loader2 className="size-4 animate-spin" />}
                {t("import.run")}
              </Button>
              <Button variant="ghost" onClick={() => setImporting(false)}>
                {t("common:cancel", "Cancel")}
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {data.loans.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-2 py-10 text-center text-muted-foreground">
            <Banknote className="size-8 opacity-50" />
            <p>{t("empty")}</p>
          </CardContent>
        </Card>
      ) : (
        <div className="flex flex-col gap-3">
          {data.loans.map((loan) => {
            const hasLock = (loan.legs ?? []).some((leg) =>
              ["collateral_lock", "collateral_topup"].includes(leg.role),
            );
            return (
              <Card key={loan.id}>
                <CardHeader className="flex flex-row items-start justify-between gap-3">
                  <div>
                    <CardTitle className="text-base">
                      {loan.platform || t("untitled")}
                    </CardTitle>
                    <CardDescription>
                      {t(`roles.${loan.role}`, loan.role)} ·{" "}
                      {t(`custody.${loan.custody_type}`, loan.custody_type ?? t("custody.unset"))}
                      {loan.rehypothecation === "allowed" && (
                        <Badge variant="destructive" className="ml-2">
                          {t("rehyp")}
                        </Badge>
                      )}
                    </CardDescription>
                  </div>
                  <Select value={loan.status} onValueChange={(s) => markStatus(loan.id, s)}>
                    <SelectTrigger className="w-40">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {data.enums.statuses.map((s) => (
                        <SelectItem key={s} value={s}>
                          {t(`statuses.${s}`, s)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </CardHeader>
                <CardContent className="flex flex-col gap-3">
                  {(loan.advisory ?? []).length > 0 && (
                    <ul className="flex flex-col gap-1 rounded-md border border-amber-500/30 bg-amber-500/5 p-2 text-xs text-muted-foreground">
                      {(loan.advisory ?? []).map((note, idx) => (
                        <li key={idx} className="flex items-start gap-1.5">
                          <Info className="mt-0.5 size-3 shrink-0 text-amber-500" />
                          {note}
                        </li>
                      ))}
                    </ul>
                  )}
                  {(loan.legs ?? []).length > 0 && (
                    <ul className="flex flex-col gap-1 text-sm text-muted-foreground">
                      {(loan.legs ?? []).map((leg) => (
                        <li key={leg.id} className="flex items-center gap-2">
                          <Badge variant="secondary">{t(`legRoles.${leg.role}`, leg.role)}</Badge>
                          {leg.transaction_id && (
                            <span className="truncate font-mono text-xs">{leg.transaction_id}</span>
                          )}
                        </li>
                      ))}
                    </ul>
                  )}
                  {!hasLock && (
                    <div className="flex items-end gap-2">
                      <div className="flex flex-1 flex-col gap-1.5">
                        <Label className="text-xs">{t("lock.label")}</Label>
                        <Input
                          value={lockTxid[loan.id] ?? ""}
                          placeholder={t("lock.placeholder")}
                          onChange={(e) =>
                            setLockTxid((prev) => ({ ...prev, [loan.id]: e.target.value }))
                          }
                        />
                      </div>
                      <Button
                        variant="secondary"
                        disabled={addLeg.isPending}
                        onClick={() => submitLock(loan.id)}
                      >
                        {t("lock.add")}
                      </Button>
                    </div>
                  )}
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
