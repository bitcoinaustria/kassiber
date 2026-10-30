import { BookNetworkSettings } from "./BookNetworkSettings";

import { useTranslation } from "react-i18next";
import { ANALYSIS_NETWORKS, type AnalysisNetwork, useUiStore } from "@/store/ui";

import { Button } from "@/components/ui/button";
import {
  SettingsDisclosure,
  SettingsGroup,
  SettingsRow,
  SettingsSelect,
} from "./SettingsControls";
import {
  backendExplorerBaseUrl,
  backendProtocolLabel,
  backendTrust,
  backendsForLayer,
  explorerHostLabel,
  type Backend,
  type NetworkLayer,
} from "./SettingsModel";

/** Where transaction links go until an Explorer API backend is added. */
const PUBLIC_EXPLORER: Record<"bitcoin" | "liquid", string> = {
  bitcoin: "mempool.bitcoin-austria.at",
  liquid: "Liquid Network",
};

export function NetworkLayerSettingsPanel({
  layer,
  backends,
  onAdd,
  onEdit,
  onSetDefault,
  settingDefaultBackendId,
  publicFallbacks = true,
}: {
  layer: NetworkLayer;
  backends: Backend[];
  onAdd: () => void;
  onEdit: (backend: Backend) => void;
  onSetDefault: (backend: Backend) => void;
  settingDefaultBackendId?: string | null;
  publicFallbacks?: boolean;
}) {
  const { t } = useTranslation("settings");
  const layerBackends = backendsForLayer(backends, layer);
  const explorerLinkBase =
    layer === "lightning"
      ? null
      : layerBackends.map(backendExplorerBaseUrl).find(Boolean) ?? null;
  const linksNote =
    layer === "lightning"
      ? t("networkLayer.readOnly")
      : explorerLinkBase
        ? t("networkLayer.explorerLinks", { host: explorerHostLabel(explorerLinkBase) })
        : publicFallbacks
          ? t("networkLayer.publicLinks", { host: PUBLIC_EXPLORER[layer] })
          : t("networkLayer.noPublicLinks");
  return (
    <div className="space-y-6">
      {layer === "bitcoin" ? <BookNetworkSettings><AnalysisNetworkSettings /></BookNetworkSettings> : null}

      <SettingsGroup
        title={t(`networkLayer.${layer}.heading`)}
        actions={
          <Button type="button" size="sm" variant="outline" onClick={onAdd}>
            {t(`networkLayer.${layer}.add`)}
          </Button>
        }
        footer={`${t(`networkLayer.${layer}.blurb`)} ${linksNote}`}
      >
        {layerBackends.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            {t(`networkLayer.${layer}.empty`)}
          </p>
        ) : (
          layerBackends.map((backend) => (
            <BackendLayerRow
              key={backend.id}
              backend={backend}
              onEdit={() => onEdit(backend)}
              onSetDefault={() => onSetDefault(backend)}
              setDefaultPending={settingDefaultBackendId === backend.id}
              setDefaultDisabled={settingDefaultBackendId !== null}
            />
          ))
        )}
      </SettingsGroup>
    </div>
  );
}

export function BackendLayerRow({
  backend,
  onEdit,
  onSetDefault,
  setDefaultPending = false,
  setDefaultDisabled = false,
}: {
  backend: Backend;
  onEdit: () => void;
  onSetDefault: () => void;
  setDefaultPending?: boolean;
  setDefaultDisabled?: boolean;
}) {
  const { t } = useTranslation(["settings", "common"]);
  const trust = backendTrust(backend);
  const explorerBaseUrl = backendExplorerBaseUrl(backend);
  const summary = [
    backend.isDefault ? t("networkLayer.default") : null,
    backendProtocolLabel(backend),
    trust.label,
    explorerBaseUrl
      ? t("networkLayer.links", { host: explorerHostLabel(explorerBaseUrl) })
      : null,
    backend.silentPayments ? t("networkLayer.silentPayments") : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <SettingsRow
      label={backend.name}
      description={
        <>
          <span className="block" title={trust.note}>
            {summary}
          </span>
          <span className="block truncate font-mono text-xs">{backend.url}</span>
        </>
      }
    >
      {!backend.on ? (
        <span className="text-sm text-amber-600 dark:text-amber-400">
          {t("networkLayer.noEndpoint")}
        </span>
      ) : null}
      {!backend.isDefault ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={setDefaultDisabled}
          onClick={onSetDefault}
        >
          {setDefaultPending
            ? t("networkLayer.settingDefault")
            : t("networkLayer.setDefault")}
        </Button>
      ) : null}
      <Button
        type="button"
        size="sm"
        variant="outline"
        aria-label={t("networkLayer.editAria", { name: backend.name })}
        onClick={onEdit}
      >
        {t("common:actions.edit")}
      </Button>
    </SettingsRow>
  );
}

/** Offline defaults never alter connection networks or authorize requests. */
function AnalysisNetworkSettings() {
  const { t } = useTranslation("settings");
  const network = useUiStore((state) => state.analysisNetwork);
  const setNetwork = useUiStore((state) => state.setAnalysisNetwork);
  return (
    <SettingsDisclosure title={t("analysisNetwork.title")}>
      <SettingsRow
        label={t("analysisNetwork.label")}
        description={t("analysisNetwork.help")}
        htmlFor="analysis-network"
      >
        <SettingsSelect<AnalysisNetwork>
          value={network}
          onValueChange={setNetwork}
          options={ANALYSIS_NETWORKS.map((value) => ({ value, label: value }))}
          triggerProps={{ id: "analysis-network" }}
        />
      </SettingsRow>
    </SettingsDisclosure>
  );
}
