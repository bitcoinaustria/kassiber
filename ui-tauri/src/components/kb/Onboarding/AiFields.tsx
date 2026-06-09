import { Sparkles } from "lucide-react";

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
  const localSelected = form.aiSetupMode === "local";
  const remoteSelected = form.aiSetupMode === "remote";
  const disabledSelected = form.aiSetupMode === "disabled";
  const endpointHint = disabledSelected ? null : aiBaseUrlHint(form.aiBaseUrl);

  return (
    <div className="space-y-5">
      <div className="space-y-3">
        <ChoiceCard
          active={localSelected}
          title="Use a local endpoint"
          description="Default to Ollama on this machine. You can point this at another local OpenAI-compatible server."
          onClick={() => {
            update("aiSetupMode", "local");
            update("aiProviderKind", "local");
            update("aiProviderName", DEFAULT_AI_PROVIDER_NAME);
            update("aiBaseUrl", DEFAULT_AI_BASE_URL);
          }}
        />
        <ChoiceCard
          active={remoteSelected}
          title="Use a remote provider"
          description="Prepare an OpenAI-compatible endpoint or Claude/Codex CLI. Prompts and accounting context may leave this device."
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
          title="Disable AI for now"
          description="Hide the Assistant and floating chat. You can turn AI back on later in Settings."
          onClick={() => update("aiSetupMode", "disabled")}
        />
      </div>

      {(localSelected || remoteSelected) && (
        <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
          {remoteSelected && (
            <SelectField
              label="Provider privacy"
              value={form.aiProviderKind}
              options={AI_PROVIDER_KINDS}
              description="TEE means the provider claims a trusted-execution path; Kassiber still treats it as off-device."
              onChange={(value) => update("aiProviderKind", value)}
            />
          )}
          <TextField
            label="Provider name"
            name="aiProviderName"
            value={form.aiProviderName}
            placeholder={localSelected ? "ollama" : "openai or maple"}
            description="A short label shown in AI provider settings."
            onChange={(value) => update("aiProviderName", value)}
          />
          <TextField
            label="Base URL"
            name="aiBaseUrl"
            value={form.aiBaseUrl}
            placeholder={
              localSelected
                ? DEFAULT_AI_BASE_URL
                : "https://.../v1 or claude-cli://default"
            }
            hint={endpointHint}
            description="Use an OpenAI-compatible /v1 endpoint or a supported CLI locator."
            onChange={(value) => update("aiBaseUrl", value)}
          />
          {remoteSelected && (
            <CheckRow
              id="ai-remote-ack"
              checked={form.aiRemoteAcknowledged}
              onCheckedChange={(checked) =>
                update("aiRemoteAcknowledged", checked)
              }
              label="I understand prompts may leave this device."
              description="Accounting context, labels, notes, backend hostnames, and report details can be sensitive."
            />
          )}
        </div>
      )}

      {disabledSelected && (
        <div className="flex items-start gap-3 rounded-lg border border-accent bg-[rgba(227,0,15,0.04)] p-4 text-xs leading-5 text-ink-2">
          <Sparkles className="mt-0.5 size-4 shrink-0 text-accent" />
          <p className="m-0">
            This hides the Assistant screen and floating chat after onboarding.
            You can enable AI features later in Settings.
          </p>
        </div>
      )}
    </div>
  );
};
