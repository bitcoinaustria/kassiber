import { AlertTriangle, Globe2, KeyRound, ServerCog } from "lucide-react";

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
  BACKEND_KIND_LABELS,
  BACKEND_KINDS,
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
  PUBLIC_BACKEND_DEFAULTS,
  backendEndpointDescription,
  backendEndpointHint,
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
  BackendPreviewRow,
  OnboardingForm,
  StepComponentProps,
} from "../types";

const ConnectionsPanel = ({ form }: { form: OnboardingForm }) => {
  const modeLabel =
    form.backendSetupMode === "default"
      ? "Built-in backends"
      : form.backendSetupMode === "custom"
        ? "Custom backend"
        : "Skipped";
  const activeRows: readonly BackendPreviewRow[] =
    form.backendSetupMode === "default"
      ? PUBLIC_BACKEND_DEFAULTS
      : form.backendSetupMode === "custom"
        ? [
            {
              name: form.backendName.trim() || "custom",
              kind: BACKEND_KIND_LABELS[form.backendKind],
              url: form.backendUrl.trim() || "endpoint pending",
            },
          ]
        : [{ name: "None", kind: "Manual import", url: "configure later" }];

  return (
    <div className="flex h-full items-center">
      <div className="w-full max-w-lg rounded-lg border border-line bg-paper p-5 shadow-sm">
        <div className="flex items-center gap-3">
          <div
            className={cn(
              "flex size-10 items-center justify-center rounded-md text-paper",
              form.backendSetupMode === "skip" ? "bg-accent" : "bg-ink",
            )}
          >
            {form.backendSetupMode === "skip" ? (
              <AlertTriangle className="size-5" />
            ) : form.backendSetupMode === "custom" ? (
              <ServerCog className="size-5" />
            ) : (
              <Globe2 className="size-5" />
            )}
          </div>
          <div>
            <p className="font-semibold text-ink">{modeLabel}</p>
            <p className="text-xs text-ink-2">
              {form.backendSetupMode === "skip"
                ? "No live sync until settings are configured."
                : "Endpoint choices only; credentials stay out of onboarding."}
            </p>
          </div>
        </div>

        <div className="mt-5 overflow-hidden rounded-lg border border-line">
          <Table>
            <TableHeader>
              <TableRow className="bg-paper-2">
                {["Name", "Kind", "Endpoint"].map((head) => (
                  <TableHead key={head} className="h-9 border-r last:border-r-0">
                    {head}
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {activeRows.map((row) => (
                <TableRow key={row.name} className="even:bg-paper-2/60">
                  <TableCell className="h-10 border-r font-medium">
                    {row.name}
                  </TableCell>
                  <TableCell className="h-10 border-r">{row.kind}</TableCell>
                  <TableCell className="h-10 max-w-[240px] truncate">
                    {row.url}
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

export const ConnectionsStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const skipSelected = form.backendSetupMode === "skip";
  const customSelected = form.backendSetupMode === "custom";
  const endpointHint = customSelected
    ? backendEndpointHint(form.backendKind, form.backendUrl)
    : null;
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose sync connections"
        eyebrow="Connections"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-5">
            <div className="space-y-3">
              <ChoiceCard
                active={form.backendSetupMode === "default"}
                title="Use built-in public backends"
                description="Start quickly with the bundled Esplora, Electrum, and Liquid endpoints. You can replace them later."
                onClick={() => {
                  update("backendSetupMode", "default");
                  update("backendKind", "esplora");
                  update("backendName", DEFAULT_BACKEND_NAME);
                  update("backendUrl", DEFAULT_BACKEND_URL);
                }}
              />
              <ChoiceCard
                active={customSelected}
                title="Use a custom backend"
                description="Point Kassiber at your own node, Esplora, Electrum server, BTCPay server, or custom endpoint."
                onClick={() => {
                  update("backendSetupMode", "custom");
                  if (
                    form.backendName === DEFAULT_BACKEND_NAME &&
                    form.backendUrl === DEFAULT_BACKEND_URL
                  ) {
                    update("backendName", "");
                    update("backendUrl", "");
                  }
                }}
              />
              <ChoiceCard
                active={skipSelected}
                title="Skip connections for now"
                description="Continue with manual imports only. Wallet sync can be configured from Settings later."
                tone="warning"
                onClick={() => update("backendSetupMode", "skip")}
              />
            </div>

            {customSelected && (
              <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
                <SelectField
                  label="Backend kind"
                  value={form.backendKind}
                  options={BACKEND_KINDS}
                  description="Credentials are intentionally not collected in onboarding."
                  onChange={(value) => update("backendKind", value)}
                />
                <TextField
                  label="Display name"
                  name="backendName"
                  value={form.backendName}
                  placeholder="home-node"
                  description="A short label shown in Settings and wallet sync screens."
                  onChange={(value) => update("backendName", value)}
                />
                <TextField
                  label="Endpoint URL"
                  name="backendUrl"
                  value={form.backendUrl}
                  placeholder="https://... or ssl://..."
                  hint={endpointHint}
                  description={backendEndpointDescription(form.backendKind)}
                  onChange={(value) => update("backendUrl", value)}
                />
                <div className="flex items-start gap-3 rounded-lg border border-line bg-paper p-3 text-xs leading-5 text-ink-2">
                  <KeyRound className="mt-0.5 size-4 shrink-0 text-ink" />
                  <p className="m-0">
                    Do not paste API tokens, RPC passwords, cookies, or bearer
                    headers here. Credentials should be added only after the
                    encrypted database is open.
                  </p>
                </div>
              </div>
            )}

            {skipSelected && (
              <div className="space-y-3 rounded-lg border border-accent bg-[rgba(227,0,15,0.04)] p-4">
                <div className="flex items-start gap-3">
                  <AlertTriangle className="mt-0.5 size-5 shrink-0 text-accent" />
                  <div>
                    <p className="m-0 font-semibold text-ink">
                      Wallet sync will not be ready.
                    </p>
                    <p className="m-0 mt-1 text-xs leading-5 text-ink-2">
                      You can still import files, but address discovery,
                      BTCPay live history, and node-backed sync remain disabled
                      until a backend is configured.
                    </p>
                  </div>
                </div>
                <CheckRow
                  id="skip-backends-ack"
                  checked={form.skipBackendsAcknowledged}
                  onCheckedChange={(checked) =>
                    update("skipBackendsAcknowledged", checked)
                  }
                  label="I understand sync needs a backend later."
                  description="Settings can add built-in or custom backends after onboarding."
                />
              </div>
            )}
          </div>

          <Button onClick={onSubmit} className="w-full" disabled={!canContinue}>
            Continue
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <ConnectionsPanel form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
