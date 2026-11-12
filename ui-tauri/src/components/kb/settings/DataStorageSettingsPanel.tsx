import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { SettingsCopyRow, SettingsGroup, SettingsRow } from "./SettingsControls";
import { useUiStore } from "@/store/ui";
import { BackupSettings } from "./BackupSettings";
import { formatBytes, type StatusData, type StatusStorage } from "./SettingsModel";

export function DataSettingsPanel({
  status,
  storage = null,
  onOpenImports,
}: {
  status: StatusData | null;
  /** Database file sizes; null while loading or when unavailable. */
  storage?: StatusStorage | null;
  onOpenImports: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const daemonSession = useUiStore((state) => state.daemonSession);
  return (
    <div className="space-y-6">
      <SettingsGroup>
        <SettingsRow
          anchorId="importData"
          label={t("data.importHeading")}
          description={t("data.importDescription")}
        >
          <Button type="button" variant="outline" size="sm" onClick={onOpenImports}>
            {t("data.openImportTools")}
          </Button>
        </SettingsRow>
      </SettingsGroup>

      <BackupSettings key={daemonSession} />

      <SettingsGroup title={t("data.localDatabaseHeading")}>
        <SettingsCopyRow
          anchorId="dataRoot"
          label={t("data.dataRootLabel")}
          value={status?.data_root ?? null}
          copyLabel={t("page.copy", { label: t("data.dataRootLabel") })}
          placeholder={t("common:state.loading")}
        />
        <SettingsCopyRow
          anchorId="databasePath"
          label={t("data.databaseLabel")}
          value={status?.database ?? null}
          copyLabel={t("page.copy", { label: t("data.databaseLabel") })}
          placeholder={t("common:state.loading")}
        />
        <SettingsRow
          anchorId="databaseSize"
          label={t("data.databaseSizeLabel")}
          description={
            // A write-ahead log is noted only when it holds anything.
            storage?.wal_bytes
              ? t("data.databaseSizeWal", { size: formatBytes(storage.wal_bytes) })
              : undefined
          }
        >
          <span className="text-sm tabular-nums text-muted-foreground">
            {storage?.database_bytes != null
              ? formatBytes(storage.database_bytes)
              : storage
                ? t("data.databaseSizeUnknown")
                : t("common:state.loading")}
          </span>
        </SettingsRow>
      </SettingsGroup>
    </div>
  );
}

export function DataStorageSettingsPanel({
  status,
  storage = null,
  onOpenImports,
  onResetWelcome,
  onResetBook,
  resetBookDisabled,
  onDeleteBooks,
  deleteBooksDisabled,
  resetRegtestAvailable = false,
  onResetRegtest,
  resetRegtestPending = false,
}: {
  status: StatusData | null;
  storage?: StatusStorage | null;
  onOpenImports: () => void;
  onResetWelcome: () => void;
  onResetBook: () => void;
  resetBookDisabled: boolean;
  onDeleteBooks: () => void;
  deleteBooksDisabled: boolean;
  resetRegtestAvailable?: boolean;
  onResetRegtest?: () => void;
  resetRegtestPending?: boolean;
}) {
  const { t } = useTranslation("settings");
  return (
    <div className="space-y-6">
      <DataSettingsPanel
        status={status}
        storage={storage}
        onOpenImports={onOpenImports}
      />

      {/* Each action asks for confirmation before anything is cleared. */}
      <SettingsGroup title={t("data.dangerHeading")}>
        <SettingsRow
          anchorId="resetWelcome"
          label={t("data.resetWelcomeTitle")}
          description={t("data.resetWelcomeDescription")}
        >
          <Button type="button" variant="outline" size="sm" onClick={onResetWelcome}>
            {t("data.resetWelcomeButton")}
          </Button>
        </SettingsRow>
        <SettingsRow
          anchorId="resetBook"
          label={t("data.resetBookTitle")}
          description={t("data.resetBookDescription")}
        >
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={resetBookDisabled}
            onClick={onResetBook}
          >
            {t("data.resetBookButton")}
          </Button>
        </SettingsRow>
        <SettingsRow
          anchorId="deleteBooks"
          label={t("data.deleteBooksTitle")}
          description={t("data.deleteBooksDescription")}
        >
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="hover:text-destructive"
            disabled={deleteBooksDisabled}
            onClick={onDeleteBooks}
          >
            {t("data.deleteBooksButton")}
          </Button>
        </SettingsRow>
        {resetRegtestAvailable ? (
          <SettingsRow
            label={t("data.resetRegtestTitle")}
            description={t("data.resetRegtestDescription")}
          >
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="hover:text-destructive"
              disabled={resetRegtestPending}
              onClick={onResetRegtest}
            >
              {resetRegtestPending
                ? t("data.resetRegtestPending")
                : t("data.resetRegtestButton")}
            </Button>
          </SettingsRow>
        ) : null}
      </SettingsGroup>
    </div>
  );
}
