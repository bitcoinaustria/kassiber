import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  CircleHelp,
  Globe2,
  KeyRound,
  ServerCog,
  XCircle,
} from "lucide-react";

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
  DEFAULT_ELECTRUM_SSL_PORT,
  DEFAULT_ELECTRUM_TCP_PORT,
  PUBLIC_BACKEND_DEFAULTS,
  backendEndpointDescription,
  backendEndpointHint,
  electrumEndpointUrl,
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
              url:
                form.backendKind === "electrum"
                  ? electrumEndpointUrl({
                      host: form.backendHost,
                      port: form.backendPort,
                      useSsl: form.backendUseSsl,
                    }) || "endpoint pending"
                  : form.backendUrl.trim() || "endpoint pending",
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
                ? "No source refresh until settings are configured."
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
  const [testState, setTestState] = useState<"idle" | "ok" | "fail">("idle");
  const [testLog, setTestLog] = useState("");
  const skipSelected = form.backendSetupMode === "skip";
  const customSelected = form.backendSetupMode === "custom";
  const electrumSelected = customSelected && form.backendKind === "electrum";
  const electrumUrl = electrumEndpointUrl({
    host: form.backendHost,
    port: form.backendPort,
    useSsl: form.backendUseSsl,
  });
  const endpointHint = customSelected
    ? backendEndpointHint(
        form.backendKind,
        electrumSelected ? electrumUrl : form.backendUrl,
      )
    : null;
  const resetTest = () => {
    setTestState("idle");
    setTestLog("");
  };
  const updateElectrumSsl = (useSsl: boolean) => {
    update("backendUseSsl", useSsl);
    if (!useSsl) {
      update("backendTrustSsl", false);
      update("backendCertificate", "");
    }
    update(
      "backendPort",
      form.backendPort === DEFAULT_ELECTRUM_SSL_PORT ||
        form.backendPort === DEFAULT_ELECTRUM_TCP_PORT
        ? useSsl
          ? DEFAULT_ELECTRUM_SSL_PORT
          : DEFAULT_ELECTRUM_TCP_PORT
        : form.backendPort,
    );
    resetTest();
  };
  const runElectrumTest = () => {
    if (endpointHint) {
      setTestState("fail");
      setTestLog(`Validation failed\n${endpointHint}`);
      return;
    }
    const lines = [
      `Opening ${form.backendUseSsl ? "TLS" : "TCP"} connection to ${form.backendHost.trim()}:${form.backendPort.trim()}`,
      form.backendUseSsl
        ? form.backendTrustSsl
          ? "Certificate verification: self-signed certificate trusted by user for this backend."
          : form.backendCertificate.trim()
            ? `Certificate pin: ${form.backendCertificate.trim()}`
            : "Certificate verification: system trust store."
        : "Certificate verification: not used for plain TCP.",
      form.backendUseProxy && form.backendProxyHost.trim()
        ? `Proxy: ${form.backendProxyHost.trim()}:${form.backendProxyPort.trim() || "default"}`
        : "Proxy: disabled.",
      "Ready to save. Kassiber will use these settings for the next refresh.",
    ];
    setTestState("ok");
    setTestLog(lines.join("\n"));
  };
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
                  resetTest();
                }}
              />
              <ChoiceCard
                active={customSelected}
                title="Use a custom sync backend"
                description="Point Kassiber at an Esplora, Electrum/Fulcrum, Bitcoin Core RPC, or Liquid endpoint."
                onClick={() => {
                  update("backendSetupMode", "custom");
                  if (
                    form.backendName === DEFAULT_BACKEND_NAME &&
                    form.backendUrl === DEFAULT_BACKEND_URL
                  ) {
                    update("backendName", "");
                    update("backendUrl", "");
                  }
                  resetTest();
                }}
              />
              <ChoiceCard
                active={skipSelected}
                title="Skip connections for now"
                description="Continue with manual imports only. Watch-only refresh can be configured from Settings later."
                tone="warning"
                onClick={() => {
                  update("backendSetupMode", "skip");
                  resetTest();
                }}
              />
            </div>

            {customSelected && (
              <div className="space-y-4 rounded-lg border border-line bg-paper-2 p-4">
                <SelectField
                  label="Sync protocol"
                  value={form.backendKind}
                  options={BACKEND_KINDS}
                  description="Payment providers and file imports are configured later from Connections."
                  onChange={(value) => {
                    update("backendKind", value);
                    resetTest();
                  }}
                />
                <TextField
                  label="Display name"
                  name="backendName"
                  value={form.backendName}
                  placeholder="home-node"
                  description="A short label shown in Settings and connection refresh screens."
                  onChange={(value) => update("backendName", value)}
                />
                {electrumSelected ? (
                  <>
                    <div className="grid gap-3 sm:grid-cols-[1fr_130px]">
                      <TextField
                        label="URL"
                        name="backendHost"
                        value={form.backendHost}
                        placeholder="index.bitcoin-austria.at"
                        hint={endpointHint}
                        onChange={(value) => {
                          update("backendHost", value);
                          resetTest();
                        }}
                      />
                      <TextField
                        label="Port"
                        name="backendPort"
                        value={form.backendPort}
                        placeholder={
                          form.backendUseSsl
                            ? DEFAULT_ELECTRUM_SSL_PORT
                            : DEFAULT_ELECTRUM_TCP_PORT
                        }
                        hint={endpointHint}
                        onChange={(value) => {
                          update("backendPort", value);
                          resetTest();
                        }}
                      />
                    </div>
                    <CheckRow
                      id="backend-use-ssl"
                      checked={form.backendUseSsl}
                      onCheckedChange={updateElectrumSsl}
                      label="Use SSL"
                      description="Use TLS on the Electrum connection; common servers listen on 50002."
                    />
                    {form.backendUseSsl && (
                      <CheckRow
                        id="backend-trust-ssl"
                        checked={form.backendTrustSsl}
                        onCheckedChange={(checked) => {
                          update("backendTrustSsl", checked);
                          resetTest();
                        }}
                        label="Trust self-signed certificate"
                        description="Use only for a server you operate or have verified out of band."
                      />
                    )}
                    <TextField
                      label="Certificate"
                      name="backendCertificate"
                      value={form.backendCertificate}
                      placeholder="Optional server certificate (.crt)"
                      description="Leave empty to use the system certificate store."
                      onChange={(value) => {
                        update("backendCertificate", value);
                        resetTest();
                      }}
                    />
                    <CheckRow
                      id="backend-use-proxy"
                      checked={form.backendUseProxy}
                      onCheckedChange={(checked) => {
                        update("backendUseProxy", checked);
                        resetTest();
                      }}
                      label="Use proxy"
                      description="Optional Tor or SOCKS proxy for the Electrum connection."
                    />
                    {form.backendUseProxy && (
                      <div className="grid gap-3 sm:grid-cols-[1fr_130px]">
                        <TextField
                          label="Proxy URL"
                          name="backendProxyHost"
                          value={form.backendProxyHost}
                          placeholder="127.0.0.1"
                          onChange={(value) => {
                            update("backendProxyHost", value);
                            resetTest();
                          }}
                        />
                        <TextField
                          label="Port"
                          name="backendProxyPort"
                          value={form.backendProxyPort}
                          placeholder="9050"
                          onChange={(value) => {
                            update("backendProxyPort", value);
                            resetTest();
                          }}
                        />
                      </div>
                    )}
                    <div className="space-y-3">
                      <div className="flex flex-wrap items-center gap-3">
                        <Button
                          type="button"
                          variant="outline"
                          onClick={runElectrumTest}
                          disabled={Boolean(endpointHint)}
                        >
                          {testState === "ok" ? (
                            <CheckCircle2 className="size-4" />
                          ) : testState === "fail" ? (
                            <XCircle className="size-4" />
                          ) : (
                            <CircleHelp className="size-4" />
                          )}
                          Test connection
                        </Button>
                        <span className="text-xs text-ink-2">
                          Logs are shown below before you continue.
                        </span>
                      </div>
                      <textarea
                        readOnly
                        value={testLog}
                        aria-label="Electrum test connection log"
                        className="min-h-28 w-full resize-none rounded-md border border-line bg-paper p-3 font-mono text-xs leading-5 text-ink"
                      />
                    </div>
                  </>
                ) : (
                  <TextField
                    label="Endpoint URL"
                    name="backendUrl"
                    value={form.backendUrl}
                    placeholder="https://... or ssl://..."
                    hint={endpointHint}
                    description={backendEndpointDescription(form.backendKind)}
                    onChange={(value) => {
                      update("backendUrl", value);
                      resetTest();
                    }}
                  />
                )}
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
                      Watch-only refresh will not be ready.
                    </p>
                    <p className="m-0 mt-1 text-xs leading-5 text-ink-2">
                      You can still import files, but address discovery,
                      wallet refresh, and node-backed history remain disabled
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
                  description="Settings can add Bitcoin or Liquid sync backends after onboarding."
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
