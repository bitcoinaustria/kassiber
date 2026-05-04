import { BrainCircuit, Power, ShieldAlert, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

import {
  AI_PROVIDER_KIND_LABELS,
  DEFAULT_AI_BASE_URL,
  DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrlHint,
} from "../constants";
import {
  CheckRow,
  ChoiceCard,
  SelectField,
  TextField,
} from "../fields";
import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type {
  AiProviderKind,
  OnboardingForm,
  StepComponentProps,
} from "../types";

const AI_PROVIDER_KINDS: AiProviderKind[] = ["remote", "tee"];

const AiPanel = ({ form }: { form: OnboardingForm }) => {
  const disabled = form.aiSetupMode === "disabled";
  const remote = form.aiSetupMode === "remote";
  const localProviderName =
    form.aiProviderName.trim() || DEFAULT_AI_PROVIDER_NAME;
  const localBaseUrl = form.aiBaseUrl.trim() || DEFAULT_AI_BASE_URL;
  const remoteProviderName = form.aiProviderName.trim();
  const remoteBaseUrl = form.aiBaseUrl.trim();
  const rows: ReadonlyArray<readonly [string, string]> = disabled
    ? [
        ["Mode", "Disabled for now"],
        ["Assistant", "Can be enabled later"],
        ["Tools", "No onboarding consent granted"],
      ]
    : [
        [
          "Mode",
          remote ? AI_PROVIDER_KIND_LABELS[form.aiProviderKind] : "Local",
        ],
        [
          "Provider",
          remote ? remoteProviderName || "provider pending" : localProviderName,
        ],
        [
          "Base URL",
          remote ? remoteBaseUrl || "endpoint pending" : localBaseUrl,
        ],
        ["Tool actions", "Consent required"],
      ];

  return (
    <div className="flex h-full items-center">
      <div className="w-full max-w-lg rounded-lg border border-line bg-paper p-5 shadow-sm">
        <div className="flex items-center gap-3">
          <div
            className={cn(
              "flex size-10 items-center justify-center rounded-md text-paper",
              disabled ? "bg-accent" : "bg-ink",
            )}
          >
            {disabled ? (
              <Power className="size-5" />
            ) : remote ? (
              <ShieldAlert className="size-5" />
            ) : (
              <BrainCircuit className="size-5" />
            )}
          </div>
          <div>
            <p className="font-semibold text-ink">
              {disabled
                ? "AI disabled for now"
                : remote
                  ? "Remote provider"
                  : "Local endpoint"}
            </p>
            <p className="text-xs text-ink-2">
              {disabled
                ? "Assistant controls remain available after onboarding."
                : "No API keys or secrets are collected in onboarding."}
            </p>
          </div>
        </div>

        <div className="mt-5 overflow-hidden rounded-lg border border-line">
          <Table>
            <TableHeader>
              <TableRow className="bg-paper-2">
                {["Setting", "Value"].map((head) => (
                  <TableHead key={head} className="h-9 border-r last:border-r-0">
                    {head}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map(([label, value]) => (
                <TableRow key={label} className="even:bg-paper-2/60">
                  <TableCell className="h-10 border-r font-medium">
                    {label}
                  </TableCell>
                  <TableCell className="h-10 max-w-[280px] truncate">
                    {value}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>
    </div>
  );
};

export const AiStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const localSelected = form.aiSetupMode === "local";
  const remoteSelected = form.aiSetupMode === "remote";
  const disabledSelected = form.aiSetupMode === "disabled";
  const endpointHint = disabledSelected ? null : aiBaseUrlHint(form.aiBaseUrl);

  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose AI assistance"
        eyebrow="AI"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
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
                description="Prepare an OpenAI-compatible endpoint outside this app. Prompts and accounting context may leave this device."
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
            </div>

            <Button
              type="button"
              variant={disabledSelected ? "default" : "outline"}
              className="w-full justify-center"
              onClick={() => update("aiSetupMode", "disabled")}
            >
              <Power className="size-4" />
              Disable AI for now
            </Button>

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
                    localSelected ? DEFAULT_AI_BASE_URL : "https://.../v1"
                  }
                  hint={endpointHint}
                  description="Must be an OpenAI-compatible /v1 endpoint."
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
                  This records your setup preference only. The assistant
                  controls remain available after onboarding.
                </p>
              </div>
            )}
          </div>

          <Button onClick={onSubmit} className="w-full" disabled={!canContinue}>
            Continue
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <AiPanel form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
