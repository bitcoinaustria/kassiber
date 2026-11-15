import * as React from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemonMutation } from "@/daemon/client";

export type JournalVerifyDifference = {
  table: string;
  change: "changed" | "missing" | "unexpected";
  key: string;
  transaction_id?: string;
  wallet_id?: string;
  year?: string;
  columns?: { column: string; stored: unknown; recomputed: unknown }[];
};

export type JournalVerifyReport = {
  status:
    | "identical"
    | "different"
    | "stale"
    | "not_processed"
    | "no_transactions"
    | "refused";
  reason?: string;
  deterministic?: boolean | null;
  differing?: number;
  truncated?: boolean;
  by_wallet?: Record<string, number>;
  by_year?: Record<string, number>;
  differences?: JournalVerifyDifference[];
  error?: { code?: string; message?: string };
};

const LIMIT = 20;

/**
 * The reproducibility check (docs/reference/replay-bundle.md): recompute the
 * journals in a private in-memory copy and compare them with the stored
 * ones. Runs only when the user asks; it reads the book and writes nothing.
 */
export function JournalVerifyDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation(["journals", "common"]);
  const verify = useDaemonMutation<JournalVerifyReport>("ui.journals.verify", {
    invalidateQueries: false,
  });
  const { mutate, reset } = verify;
  React.useEffect(() => {
    if (open) {
      mutate({ limit: LIMIT });
    } else {
      reset();
    }
  }, [open, mutate, reset]);

  const report = verify.data?.data;
  const error = verify.error instanceof Error ? verify.error.message : null;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("ledger.verify.title")}</DialogTitle>
          <DialogDescription>{t("ledger.verify.description")}</DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] space-y-3 overflow-y-auto text-sm" role="status">
          {verify.isPending ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("ledger.verify.running")}
            </p>
          ) : null}
          {error ? <p className="text-destructive">{error}</p> : null}
          {report ? <VerifyReport report={report} /> : null}
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button">{t("common:actions.close")}</Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function VerifyReport({ report }: { report: JournalVerifyReport }) {
  const { t } = useTranslation(["journals"]);
  const headline = (() => {
    switch (report.status) {
      case "identical":
        return t("ledger.verify.status.identical");
      case "different":
        return t("ledger.verify.status.different", { count: report.differing ?? 0 });
      case "stale":
      case "not_processed":
        return t("ledger.verify.status.stale");
      case "no_transactions":
        return t("ledger.verify.status.noTransactions");
      default:
        return t("ledger.verify.status.refused", {
          message: report.error?.message ?? "",
        });
    }
  })();
  const groups = (values: Record<string, number> | undefined) =>
    Object.entries(values ?? {})
      .map(([name, count]) => `${name} (${count})`)
      .join(", ");
  return (
    <>
      <p className="font-medium">{headline}</p>
      {report.status === "different" && report.deterministic === false ? (
        <p className="text-muted-foreground">{t("ledger.verify.nondeterministic")}</p>
      ) : null}
      {report.status === "different" ? (
        <>
          {report.by_wallet && Object.keys(report.by_wallet).length ? (
            <p>{t("ledger.verify.byWallet", { wallets: groups(report.by_wallet) })}</p>
          ) : null}
          {report.by_year && Object.keys(report.by_year).length ? (
            <p>{t("ledger.verify.byYear", { years: groups(report.by_year) })}</p>
          ) : null}
          <ul className="space-y-1 font-mono text-xs">
            {(report.differences ?? []).map((difference) => (
              <li key={`${difference.table}:${difference.key}`}>
                {difference.table} · {difference.change} · {difference.key}
                {difference.columns?.length
                  ? ` · ${difference.columns.map((c) => c.column).join(", ")}`
                  : ""}
              </li>
            ))}
          </ul>
          {report.truncated ? (
            <p className="text-muted-foreground">
              {t("ledger.verify.truncated", { count: report.differences?.length ?? 0 })}
            </p>
          ) : null}
          <p className="text-muted-foreground">{t("ledger.verify.nextStep")}</p>
        </>
      ) : null}
    </>
  );
}
