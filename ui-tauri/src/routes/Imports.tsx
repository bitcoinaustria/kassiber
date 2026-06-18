/**
 * Imports route — integrations and connection onboarding as a real screen.
 */

import { useMemo, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import {
  SettingsIntegrations4,
  type IntegrationItem,
} from "@/components/shadcnblocks/settings-integrations4";
import { Button } from "@/components/ui/button";
import {
  CONNECTION_SOURCES,
  connectionCategoryLabel,
} from "@/lib/connectionCatalog";
import { screenShellClassName } from "@/lib/screen-layout";

export function Imports() {
  const { t } = useTranslation("review");
  const navigate = useNavigate();
  const [selectedId, setSelectedId] = useState("xpub");
  const [dialogSourceId, setDialogSourceId] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  const importItems = useMemo<IntegrationItem[]>(
    () =>
      CONNECTION_SOURCES.map((source) => ({
        id: source.id,
        title: source.title,
        description: source.description,
        category: source.category,
        categoryLabel: connectionCategoryLabel(source.category),
        image: source.image ?? "",
        className: source.imageClassName,
        imageFrameClassName: source.imageFrameClassName,
        actionLabel:
          source.status === "ready"
            ? source.setupKind === "backend-settings"
              ? t("imports.actionConfigure")
              : t("imports.actionSetup")
            : t("imports.actionPlanned"),
      })),
    [t],
  );

  return (
    <div className={screenShellClassName}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t("imports.eyebrow")}
          </p>
          <h2 className="text-2xl font-semibold tracking-tight">
            {t("imports.title")}
          </h2>
          <p className="text-sm text-muted-foreground">
            {t("imports.description")}
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void navigate({ to: "/connections" })}
        >
          <ArrowLeft className="size-4" aria-hidden="true" />
          {t("imports.connections")}
        </Button>
      </div>

      <SettingsIntegrations4
        heading={t("imports.sourcesHeading")}
        subHeading={t("imports.sourcesSubHeading")}
        integrations={importItems}
        selectedId={selectedId}
        onSelect={(integration) => {
          const nextId = integration.id ?? integration.title;
          setSelectedId(nextId);
          const source = CONNECTION_SOURCES.find((candidate) => candidate.id === nextId);
          if (source?.status === "ready") {
            setDialogSourceId(source.id);
            setDialogOpen(true);
          }
        }}
      />
      <AddConnectionDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        initialSourceId={dialogSourceId}
      />
    </div>
  );
}
