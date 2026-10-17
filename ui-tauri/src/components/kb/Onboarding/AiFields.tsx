import { BrainCircuit, Cloud, Power, WifiOff } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrlHint,
  localAiBaseUrlHint,
} from "./constants";
import { CheckRow, ChoiceCard, SelectField, TextField } from "./fields";
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
        description={t("ai.providerNameDescription")}
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
        detail={t("ai.local.tradeoff")}
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
        onClick={() => update("aiSetupMode", "disabled")}
      >
        <p className="text-xs leading-5 text-ink-2">{t("ai.disabledNote")}</p>
      </ChoiceCard>
    </div>
  );
};
