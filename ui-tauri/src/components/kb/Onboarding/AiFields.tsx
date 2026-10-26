import { BrainCircuit, Cloud, Power, WifiOff } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrlHint,
  localAiBaseUrlHint,
} from "./constants";
import { CheckRow, ChoiceCard, SelectField, TextField } from "./fields";
import { endpointHost, endpointReach, type NetworkSummary } from "./networkReach";
import type { AiProviderKind, OnboardingForm } from "./types";

const AI_PROVIDER_KINDS: AiProviderKind[] = ["remote", "tee"];

interface AiFieldsProps {
  form: OnboardingForm;
  update: <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => void;
}

/**
 * The AI-assistance chooser (local / remote / off) plus provider fields.
 * Extracted from the old standalone AI step so it can live inside the merged
 * Essentials step's "AI assistant" disclosure.
 */
export const AiFields = ({ form, update }: AiFieldsProps) => {
  const { t } = useTranslation("onboarding");
  if (form.backendSetupMode === "skip") {
    return (
      <div className="flex items-start gap-3 rounded-(--kb-radius-card) border border-border p-4 text-sm leading-6 text-ink-2">
        <WifiOff
          className="mt-1 size-4 shrink-0 text-ink-3"
          aria-hidden="true"
        />
        <p className="m-0">{t("ai.offlineNote")}</p>
      </div>
    );
  }
  const localSelected = form.aiSetupMode === "local";
  const remoteSelected = form.aiSetupMode === "remote";
  const disabledSelected = form.aiSetupMode === "disabled";
  const endpointHint = disabledSelected
    ? null
    : localSelected
      ? localAiBaseUrlHint(form.aiBaseUrl)
      : aiBaseUrlHint(form.aiBaseUrl);

  // What each option would contact. The URL field is shared, so an option
  // that isn't chosen shows its default rather than the other one's entry.
  const assistantNetwork = (url: string, pending: string): NetworkSummary => {
    const reach = url ? endpointReach(url) : "internet";
    return {
      reach,
      label: t("network.label", {
        reach: t(`network.reach.${reach}`),
        when: t("network.when.assistant"),
      }),
      hosts: url ? [{ host: endpointHost(url) }] : undefined,
      pending,
    };
  };
  const localUrl = localSelected ? form.aiBaseUrl.trim() : DEFAULT_AI_BASE_URL;
  const remoteUrl = remoteSelected ? form.aiBaseUrl.trim() : "";

  // The provider fields belong to whichever endpoint card is chosen.
  const providerFields = (
    <>
      {remoteSelected && (
        <SelectField
          label={t("ai.providerPrivacy")}
          value={form.aiProviderKind}
          options={AI_PROVIDER_KINDS}
          description={t("ai.providerPrivacyDescription")}
          onChange={(value) => update("aiProviderKind", value)}
        />
      )}
      <TextField
        label={t("ai.providerName")}
        name="aiProviderName"
        value={form.aiProviderName}
        placeholder={
          localSelected
            ? t("ai.providerNamePlaceholderLocal")
            : t("ai.providerNamePlaceholderRemote")
        }
        onChange={(value) => update("aiProviderName", value)}
      />
      <TextField
        label={t("ai.baseUrl")}
        name="aiBaseUrl"
        value={form.aiBaseUrl}
        placeholder={
          localSelected
            ? DEFAULT_AI_BASE_URL
            : t("ai.baseUrlPlaceholderRemote")
        }
        hint={endpointHint}
        description={t("ai.baseUrlDescription")}
        onChange={(value) => update("aiBaseUrl", value)}
      />
      {remoteSelected && (
        <CheckRow
          id="ai-remote-ack"
          checked={form.aiRemoteAcknowledged}
          onCheckedChange={(checked) =>
            update("aiRemoteAcknowledged", checked)
          }
          label={t("ai.remoteAck")}
          description={t("ai.remoteAckDescription")}
        />
      )}
    </>
  );

  return (
    <div className="space-y-3">
      <ChoiceCard
        active={localSelected}
        icon={BrainCircuit}
        title={t("ai.local.title")}
        description={t("ai.local.description")}
        network={assistantNetwork(localUrl, t("network.localPending"))}
        onClick={() => {
          update("aiSetupMode", "local");
          update("aiProviderKind", "local");
          update("aiProviderName", DEFAULT_AI_PROVIDER_NAME);
          update("aiBaseUrl", DEFAULT_AI_BASE_URL);
        }}
      >
        {providerFields}
      </ChoiceCard>
      <ChoiceCard
        active={remoteSelected}
        icon={Cloud}
        title={t("ai.remote.title")}
        description={t("ai.remote.description")}
        network={assistantNetwork(remoteUrl, t("network.remotePending"))}
        tone="warning"
        onClick={() => {
          update("aiSetupMode", "remote");
          if (form.aiProviderKind === "local") {
            update("aiProviderKind", "remote");
          }
          if (form.aiProviderName === DEFAULT_AI_PROVIDER_NAME) {
            update("aiProviderName", "");
            update("aiBaseUrl", "");
          }
        }}
      >
        {providerFields}
      </ChoiceCard>
      <ChoiceCard
        active={disabledSelected}
        icon={Power}
        title={t("ai.disabled.title")}
        description={t("ai.disabled.description")}
        network={{ reach: "none", label: t("network.none") }}
        onClick={() => update("aiSetupMode", "disabled")}
      />
    </div>
  );
};
