import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  CircleHelp,
  KeyRound,
  XCircle,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useDaemonMutation } from "@/daemon/client";

import {
  BACKEND_KINDS,
  BACKEND_KIND_LABELS,
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
  DEFAULT_ELECTRUM_SSL_PORT,
  DEFAULT_ELECTRUM_TCP_PORT,
  backendEndpointDescription,
  backendEndpointHint,
  electrumEndpointUrl,
} from "./constants";
import { CheckRow, ChoiceCard, SelectField, TextField } from "./fields";
import type { OnboardingForm } from "./types";

interface ConnectionsFieldsProps {
  form: OnboardingForm;
  update: <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => void;
}

/**
 * The sync-backend chooser (built-in / custom / skip) plus the full custom
 * Electrum/Esplora/RPC controls and a live "test connection" probe. Extracted
 * from the old standalone Connections step so it can live inside the merged
 * Essentials step's "Sync connections" disclosure without duplicating logic.
 */
export const ConnectionsFields = ({ form, update }: ConnectionsFieldsProps) => {
  const { t } = useTranslation("onboarding");
  const [testState, setTestState] = useState<
    "idle" | "testing" | "ok" | "fail"
  >("idle");
  const [testLog, setTestLog] = useState("");
  const testElectrum = useDaemonMutation<{
    ok: boolean;
    logs: string[];
  }>("ui.backends.electrum.test");
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
    if (endpointHint || !electrumUrl) {
      setTestState("fail");
      setTestLog(
        `${t("connections.validationFailed")}\n${
          endpointHint ?? t("connections.endpointRequired")
        }`,
      );
      return;
    }
    setTestState("testing");
    setTestLog("");
    const proxyHost = form.backendProxyHost.trim();
    const proxyPort = form.backendProxyPort.trim();
    void testElectrum
      .mutateAsync({
        url: electrumUrl,
        trust_self_signed: form.backendUseSsl && form.backendTrustSsl,
        certificate:
          form.backendUseSsl &&
          !form.backendTrustSsl &&
          form.backendCertificate.trim()
            ? form.backendCertificate.trim()
            : undefined,
        proxy:
          form.backendUseProxy && proxyHost && proxyPort
            ? `${proxyHost}:${proxyPort}`
            : undefined,
      })
      .then((envelope) => {
        const data = envelope.data;
        setTestState(data?.ok ? "ok" : "fail");
        setTestLog((data?.logs ?? []).join("\n"));
      })
      .catch((error) => {
        setTestState("fail");
        setTestLog(
          error instanceof Error ? error.message : t("connections.testFailed"),
        );
      });
  };

  return (
    <div className="space-y-5">
      <div className="space-y-3">
        <ChoiceCard
          active={form.backendSetupMode === "default"}
          title={t("connections.default.title")}
          description={t("connections.default.description")}
          onClick={() => {
            update("backendSetupMode", "default");
            update("backendKind", "electrum");
            update("backendName", DEFAULT_BACKEND_NAME);
            update("backendUrl", DEFAULT_BACKEND_URL);
            resetTest();
          }}
        />
        <ChoiceCard
          active={customSelected}
          title={t("connections.custom.title")}
          description={t("connections.custom.description")}
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
          title={t("connections.skip.title")}
          description={t("connections.skip.description")}
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
            label={t("connections.protocolLabel")}
            value={form.backendKind}
            options={BACKEND_KINDS}
            optionLabels={BACKEND_KIND_LABELS}
            description={t("connections.protocolDescription")}
            onChange={(value) => {
              update("backendKind", value);
              resetTest();
            }}
          />
          <TextField
            label={t("connections.displayName")}
            name="backendName"
            value={form.backendName}
            placeholder={t("connections.displayNamePlaceholder")}
            description={t("connections.displayNameDescription")}
            onChange={(value) => update("backendName", value)}
          />
          {electrumSelected ? (
            <>
              <div className="grid gap-3 sm:grid-cols-[1fr_130px]">
                <TextField
                  label={t("connections.host")}
                  name="backendHost"
                  value={form.backendHost}
                  placeholder={t("connections.hostPlaceholder")}
                  hint={endpointHint}
                  onChange={(value) => {
                    update("backendHost", value);
                    resetTest();
                  }}
                />
                <TextField
                  label={t("connections.port")}
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
                label={t("connections.useSsl")}
                description={t("connections.useSslDescription")}
              />
              {form.backendUseSsl && (
                <CheckRow
                  id="backend-trust-ssl"
                  checked={form.backendTrustSsl}
                  onCheckedChange={(checked) => {
                    update("backendTrustSsl", checked);
                    resetTest();
                  }}
                  label={t("connections.trustSsl")}
                  description={t("connections.trustSslDescription")}
                />
              )}
              <TextField
                label={t("connections.certificate")}
                name="backendCertificate"
                value={form.backendCertificate}
                placeholder={t("connections.certificatePlaceholder")}
                description={
                  form.backendTrustSsl
                    ? t("connections.certificateIgnored")
                    : t("connections.certificateSystemStore")
                }
                disabled={form.backendTrustSsl}
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
                label={t("connections.useProxy")}
                description={t("connections.useProxyDescription")}
              />
              {form.backendUseProxy && (
                <div className="grid gap-3 sm:grid-cols-[1fr_130px]">
                  <TextField
                    label={t("connections.proxyHost")}
                    name="backendProxyHost"
                    value={form.backendProxyHost}
                    placeholder={t("connections.proxyHostPlaceholder")}
                    onChange={(value) => {
                      update("backendProxyHost", value);
                      resetTest();
                    }}
                  />
                  <TextField
                    label={t("connections.port")}
                    name="backendProxyPort"
                    value={form.backendProxyPort}
                    placeholder={t("connections.proxyPortPlaceholder")}
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
                    disabled={Boolean(endpointHint) || testState === "testing"}
                  >
                    {testState === "ok" ? (
                      <CheckCircle2 className="size-4" />
                    ) : testState === "fail" ? (
                      <XCircle className="size-4" />
                    ) : (
                      <CircleHelp className="size-4" />
                    )}
                    {testState === "testing"
                      ? t("connections.testing")
                      : t("connections.testConnection")}
                  </Button>
                  <span className="text-xs text-ink-2">
                    {t("connections.testHint")}
                  </span>
                </div>
                {(testState !== "idle" || testLog) && (
                  <textarea
                    readOnly
                    value={testLog}
                    aria-label={t("connections.testLogLabel")}
                    className="min-h-28 w-full resize-none rounded-md border border-line bg-paper p-3 font-mono text-xs leading-5 text-ink"
                  />
                )}
              </div>
            </>
          ) : (
            <TextField
              label={t("connections.endpointUrl")}
              name="backendUrl"
              value={form.backendUrl}
              placeholder={t("connections.endpointUrlPlaceholder")}
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
            <p className="m-0">{t("connections.credentialsWarning")}</p>
          </div>
        </div>
      )}

      {skipSelected && (
        <div className="space-y-3 rounded-lg border border-accent bg-[rgba(227,0,15,0.04)] p-4">
          <div className="flex items-start gap-3">
            <AlertTriangle className="mt-0.5 size-5 shrink-0 text-accent" />
            <div>
              <p className="m-0 font-semibold text-ink">
                {t("connections.skipWarningTitle")}
              </p>
              <p className="m-0 mt-1 text-xs leading-5 text-ink-2">
                {t("connections.skipWarningBody")}
              </p>
            </div>
          </div>
          <CheckRow
            id="skip-backends-ack"
            checked={form.skipBackendsAcknowledged}
            onCheckedChange={(checked) =>
              update("skipBackendsAcknowledged", checked)
            }
            label={t("connections.skipAck")}
            description={t("connections.skipAckDescription")}
          />
        </div>
      )}
    </div>
  );
};
