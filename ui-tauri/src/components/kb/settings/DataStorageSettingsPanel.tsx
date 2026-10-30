import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { SettingsCopyRow, SettingsGroup, SettingsRow } from "./SettingsControls";
import { useUiStore } from "@/store/ui";
import { BackupSettings } from "./BackupSettings";
import type { StatusData } from "./SettingsModel";

export function DataSettingsPanel({
  status,
  onOpenImports,
}: {
  status: StatusData | null;
  onOpenImports: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const daemonSession = useUiStore((state) => state.daemonSession);
  return (
    <div className="space-y-6">
      <SettingsGroup>
        <SettingsRow
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
          label={t("data.dataRootLabel")}
          value={status?.data_root ?? null}
          copyLabel={t("page.copy", { label: t("data.dataRootLabel") })}
          placeholder={t("common:state.loading")}
        />
        <SettingsCopyRow
          label={t("data.databaseLabel")}
          value={status?.database ?? null}
          copyLabel={t("page.copy", { label: t("data.databaseLabel") })}
          placeholder={t("common:state.loading")}
        />
      </SettingsGroup>
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
  resetRegtestAvailable = false,
  onResetRegtest,
  resetRegtestPending = false,
}: {
  status: StatusData | null;
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
      <DataSettingsPanel status={status} onOpenImports={onOpenImports} />

      {/* Each action asks for confirmation before anything is cleared. */}
      <SettingsGroup title={t("data.dangerHeading")}>
        <SettingsRow
          label={t("data.resetWelcomeTitle")}
          description={t("data.resetWelcomeDescription")}
        >
          <Button type="button" variant="outline" size="sm" onClick={onResetWelcome}>
            {t("data.resetWelcomeButton")}
          </Button>
        </SettingsRow>
        <SettingsRow
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
