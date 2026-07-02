import { Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrlHint,
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
  const localSelected = form.aiSetupMode === "local";
  const remoteSelected = form.aiSetupMode === "remote";
  const disabledSelected = form.aiSetupMode === "disabled";
  const endpointHint = disabledSelected ? null : aiBaseUrlHint(form.aiBaseUrl);

  return (
    <div className="space-y-5">
      <div className="space-y-3">
        <ChoiceCard
          active={localSelected}
          title={t("ai.local.title")}
          description={t("ai.local.description")}
          onClick={() => {
            update("aiSetupMode", "local");
            update("aiProviderKind", "local");
            update("aiProviderName", DEFAULT_AI_PROVIDER_NAME);
            update("aiBaseUrl", DEFAULT_AI_BASE_URL);
          }}
        />
        <ChoiceCard
          active={remoteSelected}
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
        />
        <ChoiceCard
          active={disabledSelected}
          title={t("ai.disabled.title")}
          description={t("ai.disabled.description")}
          onClick={() => update("aiSetupMode", "disabled")}
        />
      </div>

      {(localSelected || remoteSelected) && (
        <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
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
        </div>
      )}

      {disabledSelected && (
        <div className="flex items-start gap-3 rounded-lg border border-[var(--kb-accent)] bg-[rgba(227,0,15,0.04)] p-4 text-xs leading-5 text-ink-2">
          <Sparkles className="mt-0.5 size-4 shrink-0 text-[var(--kb-accent)]" />
          <p className="m-0">{t("ai.disabledNote")}</p>
        </div>
      )}
    </div>
  );
};
