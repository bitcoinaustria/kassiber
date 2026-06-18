import { AlertTriangle, Archive, FileInput, HardDrive, RefreshCw } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { CommandLine, PathField } from "./SettingsControls";
import type { StatusData } from "./SettingsModel";

export function DataSettingsPanel({
  status,
  onOpenImports,
}: {
  status: StatusData | null;
  onOpenImports: () => void;
}) {
  const { t } = useTranslation("settings");
  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <FileInput className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">{t("data.importHeading")}</h3>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <p className="max-w-prose text-sm text-muted-foreground">
            {t("data.importDescription")}
          </p>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={onOpenImports}
          >
            <FileInput className="size-4" aria-hidden="true" />
            {t("data.openImportTools")}
          </Button>
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Archive className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">{t("data.backupHeading")}</h3>
        </div>
        <div className="space-y-2 rounded-md border bg-background p-4">
          <p className="max-w-prose text-sm text-muted-foreground">
            <Trans
              i18nKey="data.backupDescription"
              ns="settings"
              components={[<span className="font-mono" />]}
            />
          </p>
          <CommandLine command="kassiber backup export" />
          <CommandLine command="kassiber backup import <file.age>" />
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <HardDrive className="size-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-semibold">
            {t("data.localDatabaseHeading")}
          </h3>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <PathField
            id="settings-data-root"
            label={t("data.dataRootLabel")}
            value={status?.data_root ?? null}
          />
          <PathField
            id="settings-db-path"
            label={t("data.databaseLabel")}
            value={status?.database ?? null}
          />
        </div>
      </section>
    </div>
  );
}

export function DataStorageSettingsPanel({
  status,
  onOpenImports,
  onResetWelcome,
  onResetBook,
  resetBookDisabled,
  onDeleteBooks,
  deleteBooksDisabled,
}: {
  status: StatusData | null;
  onOpenImports: () => void;
  onResetWelcome: () => void;
  onResetBook: () => void;
  resetBookDisabled: boolean;
  onDeleteBooks: () => void;
  deleteBooksDisabled: boolean;
}) {
  const { t } = useTranslation("settings");
  return (
    <div className="space-y-6">
      <DataSettingsPanel status={status} onOpenImports={onOpenImports} />

      <section className="space-y-3 rounded-md border border-destructive/30 bg-destructive/5 p-4">
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-destructive">
            <AlertTriangle className="size-4" aria-hidden="true" />
            {t("data.dangerHeading")}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t("data.dangerDescription")}
          </p>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">{t("data.resetWelcomeTitle")}</p>
            <p className="text-sm text-muted-foreground">
              {t("data.resetWelcomeDescription")}
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={onResetWelcome}
          >
            {t("data.resetWelcomeButton")}
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">{t("data.resetBookTitle")}</p>
            <p className="text-sm text-muted-foreground">
              {t("data.resetBookDescription")}
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            disabled={resetBookDisabled}
            onClick={onResetBook}
          >
            <RefreshCw className="mr-2 size-4" aria-hidden="true" />
            {t("data.resetBookButton")}
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-destructive/30 bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium text-destructive">
              {t("data.deleteBooksTitle")}
            </p>
            <p className="text-sm text-muted-foreground">
              {t("data.deleteBooksDescription")}
            </p>
          </div>
          <Button
            type="button"
            variant="destructive"
            className="shrink-0"
            disabled={deleteBooksDisabled}
            onClick={onDeleteBooks}
          >
            {t("data.deleteBooksButton")}
          </Button>
        </div>
      </section>
    </div>
  );
}
