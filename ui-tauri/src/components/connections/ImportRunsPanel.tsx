/**
 * File-import runs for one connection, each rollback-able.
 *
 * Renders nothing when the connection has no file imports, so a synced
 * descriptor wallet gains no empty section (see the project's "signal, not
 * reassurance" rule — a panel that only ever says "no imports" is noise).
 *
 * Rollback is two steps on purpose: the plan comes from the daemon (`dry_run`)
 * so the count shown is the count that will actually be deleted, including
 * reviewed work that cascades with those rows.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

export interface ImportBatch {
  id: string;
  source_format: string;
  source_filename: string | null;
  column_map: Record<string, unknown> | null;
  imported_at: string;
  rows_inserted: number;
  rows_updated: number;
  rows_skipped: number;
  rows_present: number;
  rolled_back: boolean;
}

interface RollbackPlan {
  /** Present on the dry-run plan. */
  transactions_to_delete?: number;
  /** Present after an applied rollback. */
  transactions_deleted?: number;
  also_removed?: Record<string, number>;
  applied?: boolean;
}

export function ImportRunsPanel({ walletId }: { walletId: string }) {
  const { t, i18n } = useTranslation("connections");
  const addNotification = useUiStore((s) => s.addNotification);
  const [pending, setPending] = React.useState<ImportBatch | null>(null);
  const [plan, setPlan] = React.useState<RollbackPlan | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const batchesQuery = useDaemon<{ batches: ImportBatch[] }>("ui.imports.list", {
    wallet: walletId,
  });
  // The plan changes nothing, so it must not invalidate the daemon query cache
  // — otherwise merely opening this dialog refetches every screen's data.
  const rollbackPlan = useDaemonMutation<RollbackPlan>("ui.imports.rollback", {
    invalidateQueries: false,
  });
  const rollback = useDaemonMutation<RollbackPlan>("ui.imports.rollback");

  const batches = batchesQuery.data?.data?.batches ?? [];
  if (batches.length === 0) return null;

  const openRollback = async (batch: ImportBatch) => {
    setError(null);
    setPlan(null);
    setPending(batch);
    try {
      const envelope = await rollbackPlan.mutateAsync({
        batch: batch.id,
        dry_run: true,
      });
      setPlan(envelope.data ?? null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const confirmRollback = async () => {
    if (!pending) return;
    setError(null);
    try {
      const envelope = await rollback.mutateAsync({
        batch: pending.id,
        confirm: "DELETE",
      });
      addNotification({
        title: t("detail.imports.rolledBackTitle"),
        body: t("detail.imports.rolledBackBody", {
          count:
            envelope.data?.transactions_deleted ?? pending.rows_present,
          file: pending.source_filename ?? pending.source_format,
        }),
        tone: "success",
      });
      setPending(null);
      setPlan(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return (
    <>
      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="text-sm sm:text-base">
            {t("detail.imports.title")}
          </CardTitle>
          <CardDescription>{t("detail.imports.description")}</CardDescription>
        </CardHeader>
        <CardContent className="divide-y px-4 pt-0">
          {batches.map((batch) => (
            <div
              key={batch.id}
              className="flex flex-wrap items-center justify-between gap-2 py-3"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm font-medium">
                    {batch.source_filename ?? batch.source_format}
                  </span>
                  {batch.column_map ? (
                    <Badge variant="outline">
                      {t("detail.imports.mapped")}
                    </Badge>
                  ) : null}
                </div>
                <p className="text-xs text-muted-foreground">
                  {t("detail.imports.summary", {
                    count: batch.rows_present,
                    date: new Date(batch.imported_at).toLocaleString(i18n.language),
                  })}
                </p>
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void openRollback(batch)}
                disabled={batch.rows_present === 0}
              >
                <RotateCcw className="mr-1.5 h-3.5 w-3.5" aria-hidden="true" />
                {t("detail.imports.rollback")}
              </Button>
            </div>
          ))}
        </CardContent>
      </Card>

      <Dialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPending(null);
            setPlan(null);
            setError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("detail.imports.confirmTitle")}</DialogTitle>
            <DialogDescription>
              {t("detail.imports.confirmBody", {
                count: plan?.transactions_to_delete ?? pending?.rows_present ?? 0,
                file: pending?.source_filename ?? pending?.source_format ?? "",
              })}
            </DialogDescription>
          </DialogHeader>
          {plan?.also_removed && Object.keys(plan.also_removed).length > 0 ? (
            <p className="text-sm text-muted-foreground">
              {t("detail.imports.alsoRemoved", {
                items: Object.entries(plan.also_removed)
                  .map(([key, count]) => `${count} ${key}`)
                  .join(", "),
              })}
            </p>
          ) : null}
          {error ? (
            <p className="text-sm text-destructive">{error}</p>
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => setPending(null)}
            >
              {t("detail.imports.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => void confirmRollback()}
              disabled={rollback.isPending}
            >
              {t("detail.imports.confirmAction")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
