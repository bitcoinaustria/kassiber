/**
 * Add/edit wizard for AI provider records, laid out after T3 Code's
 * add-provider wizard (AddProviderInstanceDialog, MIT, Copyright (c) 2026
 * T3 Tools Inc.): a step track, a provider chosen from cards, then only what
 * that provider needs.
 *
 * The steps: how the provider is reached (on this device, through a vendor
 * app, or a hosted API with a key) and which one; its name, address and key;
 * for a provider off this device, what leaves it and the acknowledgement;
 * then the model, with "Test connection". Nothing contacts the provider
 * until the user presses Test connection, which issues `ai.test_connection`
 * against the entered values and saves nothing. Every step stays mounted,
 * so the rules below see the whole form whichever step is in view.
 */

import {
  Activity,
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronRight,
  Info,
  KeyRound,
  Laptop,
  Loader2,
  Plug,
  ShieldAlert,
  Sparkles,
  TerminalSquare,
  type LucideIcon,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  aiProviderTestOutcome,
  aiTestConnectionArgs,
  type AiProviderTestOutcome,
  type AiTestConnectionData,
} from "@/components/kb/aiProviderTest";
import {
  AI_PROVIDER_ROUTES,
  autoRemoteReason,
  httpOrigin,
  presetNeedsApiKey,
  presetsForRoute,
  routeForPreset,
  type AiProviderPreset,
  type AiProviderRoute,
  type AutoRemoteReason,
} from "@/components/kb/aiProviderFormModel";
import { OnboardingStepper } from "@/components/kb/Onboarding/stepper";
import { PostureBadge, ProviderGlyph } from "@/components/ai/ProviderGlyph";
import { httpModelRunsAssistant } from "@/components/ai/providerModelSearch";
import { useDaemonMutation } from "@/daemon/client";
import { getTransport } from "@/daemon/transport";
import {
  isLoopbackAiHttpUrl,
  isNativeAiProviderLocator,
  type AiApiFormat,
  type AiWireApiFormat,
} from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";

export interface AiProviderInput {
  name: string;
  display_name?: string | null;
  base_url: string;
  api_key?: string;
  default_model?: string;
  kind: "local" | "remote" | "tee";
  notes?: string;
}

export interface ExistingAiProvider extends AiProviderInput {
  has_api_key: boolean;
  secret_ref?: {
    store_id: "macos_keychain" | "windows_dpapi" | "linux_secret_service" | "sqlcipher_inline";
    state: "ok" | "missing" | "needs_reauth" | "unavailable";
  };
  acknowledged_at?: string | null;
  /** false when switched off in Settings: such a provider is never contacted. */
  enabled?: boolean;
  /** The saved wire-format setting; "auto" when absent. */
  api_format?: AiApiFormat;
  /** What the saved URL was last seen to answer. */
  detected_api_format?: AiWireApiFormat | null;
  /** The saved choice to send Chat Completions without `store: false`. */
  omit_store_field?: boolean;
}

interface AiProviderFormProps {
  open: boolean;
  initial?: ExistingAiProvider | null;
  onClose: () => void;
  onSaved?: (name: string) => void;
}

const PROVIDER_KIND_HINT_KEYS = {
  local: "aiProvider.kindHint.local",
  remote: "aiProvider.kindHint.remote",
  tee: "aiProvider.kindHint.tee",
} as const satisfies Record<AiProviderInput["kind"], string>;

const AUTO_REMOTE_NOTE_KEYS = {
  url: "aiProvider.autoRemote.url",
  cli: "aiProvider.autoRemote.cli",
} as const satisfies Record<AutoRemoteReason, string>;

const API_FORMATS: readonly AiApiFormat[] = ["auto", "responses", "chat_completions"];

type StepId = "provider" | "connect" | "privacy" | "check";

const ROUTE_ICONS: Record<AiProviderRoute, LucideIcon> = {
  local: Laptop,
  app: TerminalSquare,
  api: KeyRound,
};

/** The label of a preset that stands for any other OpenAI-compatible server. */
function customPresetLabelKey(preset: AiProviderPreset) {
  return preset.kind === "local"
    ? ("aiProvider.presetCustom" as const)
    : ("aiProvider.presetCustomApi" as const);
}

export function AiProviderForm({
  open,
  initial,
  onClose,
  onSaved,
}: AiProviderFormProps) {
  const { t } = useTranslation(["settings", "common"]);
  const [name, setName] = React.useState(initial?.display_name ?? initial?.name ?? "");
  const [baseUrl, setBaseUrl] = React.useState(initial?.base_url ?? "");
  const [apiKey, setApiKey] = React.useState(initial?.api_key ?? "");
  // The origin the typed key was entered for (null until the URL names one);
  // a key is never sent to another origin.
  const [keyOrigin, setKeyOrigin] = React.useState<string | null>(null);
  const [keyCleared, setKeyCleared] = React.useState(false);
  const [defaultModel, setDefaultModel] = React.useState(initial?.default_model ?? "");
  const [kind, setKind] = React.useState<AiProviderInput["kind"]>(initial?.kind ?? "local");
  const [notes, setNotes] = React.useState(initial?.notes ?? "");
  const [remoteAcknowledged, setRemoteAcknowledged] = React.useState(false);
  // Set when the form moved the posture to remote on its own, to say why.
  const [autoRemote, setAutoRemote] = React.useState<AutoRemoteReason | null>(null);
  const [preset, setPreset] = React.useState<AiProviderPreset | null>(null);
  const [route, setRoute] = React.useState<AiProviderRoute | null>(null);
  const [step, setStep] = React.useState<StepId>(initial ? "connect" : "provider");
  const [apiFormat, setApiFormat] = React.useState<AiApiFormat>(initial?.api_format ?? "auto");
  // Never set by the form on its own: only the user's tick sends requests
  // without Kassiber's no-retention flag.
  const [omitStoreField, setOmitStoreField] = React.useState(
    initial?.omit_store_field ?? false,
  );
  // The address the format and store choices were made for. A choice made
  // for one endpoint does not carry over to another (see
  // `syncEndpointSettings`); "" until the form names one.
  const [settingsUrl, setSettingsUrl] = React.useState(initial?.base_url?.trim() ?? "");
  const [endpointReset, setEndpointReset] = React.useState<"exemption" | "format" | null>(
    null,
  );
  // Open when a format other than auto, or the store choice, was saved, so
  // the setting is seen.
  const [advancedOpen, setAdvancedOpen] = React.useState(
    Boolean(
      (initial?.api_format && initial.api_format !== "auto") || initial?.omit_store_field,
    ),
  );
  const [error, setError] = React.useState<string | null>(null);
  // Each run remembers the values it tested (`target`). A result for values
  // the form no longer holds is never shown or saved: a format detected at
  // one URL must not be stored with another. Each run also has its own `id`,
  // so of two runs for the same values only the latest one reports.
  const [testRun, setTestRun] = React.useState<
    | { state: "idle" }
    | { state: "running"; id: number; target: string; listing: string }
    | (AiProviderTestOutcome & { id: number; target: string; listing: string })
    | { state: "fail"; id: number; target: string; listing: string; message: string }
  >({ state: "idle" });
  const lastTestRunId = React.useRef(0);

  const editing = Boolean(initial);

  React.useEffect(() => {
    if (!open) return;
    setName(initial?.display_name ?? initial?.name ?? "");
    setBaseUrl(initial?.base_url ?? "");
    setApiKey("");
    setKeyOrigin(null);
    setKeyCleared(false);
    setDefaultModel(initial?.default_model ?? "");
    setKind(initial?.kind ?? "local");
    setNotes(initial?.notes ?? "");
    setRemoteAcknowledged(false);
    setAutoRemote(null);
    setPreset(null);
    setRoute(null);
    setStep(initial ? "connect" : "provider");
    setApiFormat(initial?.api_format ?? "auto");
    setOmitStoreField(initial?.omit_store_field ?? false);
    setSettingsUrl(initial?.base_url?.trim() ?? "");
    setEndpointReset(null);
    setAdvancedOpen(
      Boolean(
        (initial?.api_format && initial.api_format !== "auto") || initial?.omit_store_field,
      ),
    );
    setError(null);
    setTestRun({ state: "idle" });
  }, [open, initial]);

  const applyPreset = (next: AiProviderPreset) => {
    setRoute(routeForPreset(next.id));
    setError(null);
    // A preset replaces the address, even when it is the one already chosen
    // (its URL may have been edited since): a key, format, store choice or
    // test result typed or found for the previous address does not carry
    // over, so nothing chosen for one endpoint is inherited by the next.
    setApiKey("");
    setKeyOrigin(null);
    setKeyCleared(false);
    setApiFormat("auto");
    setOmitStoreField(false);
    setAdvancedOpen(false);
    setTestRun({ state: "idle" });
    setEndpointReset(null);
    setSettingsUrl(next.base_url.trim());
    setPreset(next);
    setName(next.display_name);
    setBaseUrl(next.base_url);
    setKind(next.kind);
    setAutoRemote(null);
    setDefaultModel(next.default_model);
    setNotes(
      next.custom
        ? ""
        : next.kind === "local"
          ? t("aiProvider.presetNoteLocal", { label: next.label })
          : t("aiProvider.presetNoteRemote", { label: next.label }),
    );
    setRemoteAcknowledged(false);
  };

  /**
   * Reset the choices made for one endpoint when `url` is another one: the
   * format back to auto, the no-retention exemption off, and the last test
   * result gone, as the daemon resets a saved exemption with a new base URL.
   * True when an HTTP address lost a choice, so a test or save stops and the
   * note says why; a CLI target takes neither choice and is not held up.
   */
  const syncEndpointSettings = (url: string): boolean => {
    const current = url.trim();
    if (!settingsUrl) {
      // Choices made before any address belong to the first one.
      if (current) setSettingsUrl(current);
      return false;
    }
    if (current === settingsUrl) return false;
    setSettingsUrl(current);
    setTestRun({ state: "idle" });
    const lost = omitStoreField ? "exemption" : apiFormat !== "auto" ? "format" : null;
    setApiFormat("auto");
    setOmitStoreField(false);
    const shown = lost !== null && !isNativeAiProviderLocator(current) ? lost : null;
    setEndpointReset(shown);
    return shown !== null;
  };

  /**
   * Clear the typed API key when `url` names another origin than the one it
   * was typed for, and say so; true when it was cleared. A key typed before
   * the URL named an origin belongs to the first one it names.
   */
  const guardKeyOrigin = (url: string): boolean => {
    if (!apiKey.trim()) return false;
    const origin = httpOrigin(url);
    if (keyOrigin === null) {
      if (origin) setKeyOrigin(origin);
      return false;
    }
    if (origin === keyOrigin) return false;
    setApiKey("");
    setKeyOrigin(null);
    setKeyCleared(true);
    return true;
  };

  /**
   * Move a local provider to remote when `url` cannot be local, as the daemon
   * would, and return the posture that applies. A fresh acknowledgement is
   * needed after the move; the user's own remote or TEE choice is kept.
   */
  const applyUrlPosture = (url: string): AiProviderInput["kind"] => {
    const reason = autoRemoteReason(kind, url);
    if (!reason) return kind;
    setKind("remote");
    setAutoRemote(reason);
    setRemoteAcknowledged(false);
    return "remote";
  };

  const createProvider = useDaemonMutation("ai.providers.create");
  const updateProvider = useDaemonMutation("ai.providers.update");
  const setApiKeyMutation = useDaemonMutation("ai.providers.set_api_key");
  const testFingerprint = React.useMemo(
    () =>
      JSON.stringify({
        provider: editing ? (initial?.name ?? null) : null,
        baseUrl: baseUrl.trim(),
        apiKey: apiKey.trim(),
        // The test asks this model, so a different one needs a new test.
        model: defaultModel.trim(),
        // A detection found under one format setting is not the other's.
        apiFormat,
        // Nor is what the test found about the store flag.
        omitStoreField,
      }),
    [apiFormat, apiKey, baseUrl, defaultModel, editing, initial?.name, omitStoreField],
  );
  // What a model listing depends on: a listing stays useful while only the
  // model or the format changes.
  const listingFingerprint = React.useMemo(
    () =>
      JSON.stringify({
        provider: editing ? (initial?.name ?? null) : null,
        baseUrl: baseUrl.trim(),
        apiKey: apiKey.trim(),
      }),
    [apiKey, baseUrl, editing, initial?.name],
  );
  // CLI agents speak their own protocol and take no API format.
  const isCliUrl = isNativeAiProviderLocator(baseUrl);
  const formatToSave = isCliUrl ? null : apiFormat;
  // CLI agents take no store choice.
  const omitStoreToSave = isCliUrl ? null : omitStoreField;
  // OpenAI and OpenRouter take no request without a key. Only a new provider
  // is held to it: a saved one keeps its stored key when the field is blank.
  const keyRequired = presetNeedsApiKey(preset, baseUrl);
  const testStatus =
    testRun.state !== "idle" && testRun.target === testFingerprint
      ? testRun
      : ({ state: "idle" } as const);

  /**
   * Whether `posture` needs this form's off-device acknowledgement: any
   * off-device posture, unless the saved provider is already acknowledged
   * as off-device. A local provider's implicit acknowledgement never covered
   * sending prompts off the device. The daemon applies the same rule.
   */
  const ackNeededFor = (posture: AiProviderInput["kind"]) =>
    posture !== "local" &&
    (!initial || initial.kind === "local" || !initial.acknowledged_at);

  const handleTest = async () => {
    setError(null);
    if (guardKeyOrigin(baseUrl)) {
      // The key was typed for another address; it is not sent to this one.
      return;
    }
    if (syncEndpointSettings(baseUrl)) {
      // Choices made for the old address were reset; the note says which.
      return;
    }
    // Offline mode lets only local providers through, so an address off this
    // device is tested as the remote provider it will be saved as.
    const testKind = applyUrlPosture(baseUrl.trim());
    // The test sends a prompt: off this device only once acknowledged, as
    // for chat. Moving the posture just now leaves it unticked.
    const ackNeeded = ackNeededFor(testKind);
    if (ackNeeded && (!remoteAcknowledged || testKind !== kind)) {
      setError(t("aiProvider.errorAckRequiredTest"));
      setStep("privacy");
      return;
    }
    const target = testFingerprint;
    const listing = listingFingerprint;
    lastTestRunId.current += 1;
    const id = lastTestRunId.current;
    // Only the latest run may report: a newer run, even for the same values,
    // or a later edit makes this one's answer moot.
    const settle = (next: Exclude<typeof testRun, { state: "idle" | "running" }>) =>
      setTestRun((current) =>
        current.state === "running" && current.id === id ? next : current,
      );
    setTestRun({ state: "running", id, target, listing });
    try {
      const trimmedUrl = baseUrl.trim();
      if (!trimmedUrl) {
        throw new Error(t("aiProvider.errorBaseUrlRequired"));
      }
      if (!/^https?:\/\//.test(trimmedUrl) && !isNativeAiProviderLocator(trimmedUrl)) {
        throw new Error(t("aiProvider.errorUrlScheme"));
      }
      // Empty API-key field means "keep current key": naming the saved
      // provider lets the daemon use the stored value so the test exercises
      // real credentials. The default model field is the model the test asks.
      const args = aiTestConnectionArgs({
        baseUrl: trimmedUrl,
        kind: testKind,
        apiKey,
        model: defaultModel,
        provider: editing && initial ? initial.name : null,
        apiFormat: formatToSave,
        omitStoreField: omitStoreToSave,
        // The checkbox above, ticked; a saved off-device provider's own
        // acknowledgement needs no argument.
        acknowledged: ackNeeded,
      });
      const envelope = await getTransport().invoke<AiTestConnectionData>({
        kind: "ai.test_connection",
        args,
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? t("aiProvider.errorTestFailed"));
      }
      // A listing without a reply (no_model) is shown as unchecked, never
      // as success, and saves no detection.
      const outcome = aiProviderTestOutcome(envelope.data);
      settle({ ...outcome, id, target, listing });
      if (
        outcome.state === "ok" &&
        outcome.storeAccepted === false &&
        lastTestRunId.current === id
      ) {
        // Show where the explicit choice lives; nothing is ticked for the user.
        setAdvancedOpen(true);
      }
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      settle({ state: "fail", id, target, listing, message });
    }
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      if (guardKeyOrigin(baseUrl)) {
        setStep("connect");
        throw new Error(t("aiProvider.errorKeyForAnotherAddress"));
      }
      if (syncEndpointSettings(baseUrl)) {
        setStep("connect");
        throw new Error(t("aiProvider.errorSettingsResetForAddress"));
      }
      if (applyUrlPosture(baseUrl) !== kind) {
        // Moved to remote just now: the acknowledgement is still to be given.
        setStep("privacy");
        throw new Error(t("aiProvider.errorAckRequired"));
      }
      if (!editing && keyRequired && !apiKey.trim()) {
        setStep("connect");
        throw new Error(t("aiProvider.errorApiKeyRequired", { label: preset?.label ?? "" }));
      }
      if (!name.trim()) {
        setStep("connect");
        throw new Error(t("aiProvider.errorNameRequired"));
      }
      const needsRemoteAck = ackNeededFor(kind);
      if (kind === "local" && isNativeAiProviderLocator(baseUrl)) {
        throw new Error(t("aiProvider.errorCliLocalPosture"));
      }
      if (needsRemoteAck) {
        if (!remoteAcknowledged) {
          setStep("privacy");
          throw new Error(t("aiProvider.errorAckRequired"));
        }
      }
      if (editing && initial) {
        const args: Record<string, unknown> = {
          name: initial.name,
          display_name: name.trim(),
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || null,
          kind,
          notes: notes.trim() || null,
        };
        if (omitStoreToSave !== null) {
          args.omit_store_field = omitStoreToSave;
        }
        if (formatToSave) {
          args.api_format = formatToSave;
        } else {
          // A CLI agent takes no wire format: an HTTP provider moved to one
          // goes back to auto and forgets what its old endpoint answered,
          // rather than being refused for a format it keeps.
          args.api_format = "auto";
          if (initial.detected_api_format) {
            args.clear = ["detected_api_format"];
          }
        }
        if (needsRemoteAck) {
          args.acknowledged = true;
        }
        if (testStatus.state === "ok" && testStatus.detectedApiFormat) {
          // The test of these exact values found the format; a retargeted
          // provider would otherwise forget it with its old URL.
          args.detected_api_format = testStatus.detectedApiFormat;
        }
        await updateProvider.mutateAsync(args);
        if (apiKey.trim()) {
          await setApiKeyMutation.mutateAsync({
            name: initial.name,
            api_key: apiKey.trim(),
          });
        }
        onSaved?.(initial.name);
      } else {
        const args: Record<string, unknown> = {
          name: name.trim(),
          display_name: name.trim(),
          base_url: baseUrl.trim(),
          default_model: defaultModel.trim() || undefined,
          kind,
          notes: notes.trim() || undefined,
        };
        if (formatToSave) {
          args.api_format = formatToSave;
        }
        if (omitStoreToSave !== null) {
          args.omit_store_field = omitStoreToSave;
        }
        if (needsRemoteAck) {
          args.acknowledged = true;
        }
        if (testStatus.state === "ok" && testStatus.detectedApiFormat) {
          args.detected_api_format = testStatus.detectedApiFormat;
        }
        await createProvider.mutateAsync(args);
        if (apiKey.trim()) {
          await setApiKeyMutation.mutateAsync({
            name: name.trim().toLowerCase(),
            api_key: apiKey.trim(),
          });
        }
        onSaved?.(name.trim().toLowerCase());
      }
      onClose();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  };

  const isBusy = createProvider.isPending || updateProvider.isPending || setApiKeyMutation.isPending;
  // The daemon refuses to test a switched-off provider; say so up front.
  const testUnavailable =
    editing && initial?.enabled === false ? t("aiProvider.errorTestDisabled") : null;
  const needsRemoteAck = ackNeededFor(kind);
  const canSubmit = !isBusy && (!needsRemoteAck || remoteAcknowledged);
  // Said while the reason still holds; editing the URL back to this device
  // hides it, though the posture stays remote until the user picks local.
  const autoRemoteShown =
    autoRemote !== null &&
    kind === "remote" &&
    (autoRemote === "cli"
      ? isNativeAiProviderLocator(baseUrl)
      : /^https?:\/\//.test(baseUrl.trim()) && !isLoopbackAiHttpUrl(baseUrl));
  // The models the last test listed for this endpoint and key, offered in the
  // Default model field: choosing one is how a reply gets checked.
  const modelSuggestions = React.useMemo(
    () =>
      !isCliUrl &&
      (testRun.state === "ok" || testRun.state === "no_model") &&
      testRun.listing === listingFingerprint
        ? testRun.models.filter(httpModelRunsAssistant).map((model) => model.id)
        : [],
    [isCliUrl, listingFingerprint, testRun],
  );
  // A detection belongs to the URL it was made at.
  const savedDetection =
    editing && initial?.detected_api_format && baseUrl.trim() === initial.base_url.trim()
      ? initial.detected_api_format
      : null;

  // The wizard's steps. Privacy is its own step only for a provider whose
  // prompts leave this device; editing starts at the connection.
  const stepIds: StepId[] = [
    ...(editing ? [] : (["provider"] as const)),
    "connect",
    ...(kind !== "local" ? (["privacy"] as const) : []),
    "check",
  ];
  const stepIndex = Math.max(0, stepIds.indexOf(step));
  const lastStep = stepIndex === stepIds.length - 1;
  const stepSummaries: Record<StepId, string | undefined> = {
    provider: preset
      ? preset.custom
        ? t(customPresetLabelKey(preset))
        : preset.label
      : undefined,
    connect: name.trim() || undefined,
    privacy: t(`aiProvider.postureName.${kind}`),
    check: defaultModel.trim() || undefined,
  };

  /**
   * Moves on once this step holds up: a provider chosen; a name and an
   * address whose key, settings and posture are settled; the off-device
   * acknowledgement given. Nothing here contacts the provider.
   */
  const goNext = () => {
    setError(null);
    if (step === "provider") {
      if (!preset) {
        setError(t("aiProvider.errorChooseProvider"));
        return;
      }
      setStep("connect");
      return;
    }
    if (step === "connect") {
      const url = baseUrl.trim();
      if (!name.trim()) {
        setError(t("aiProvider.errorNameRequired"));
        return;
      }
      if (!url) {
        setError(t("aiProvider.errorBaseUrlRequired"));
        return;
      }
      if (!/^https?:\/\//.test(url) && !isNativeAiProviderLocator(url)) {
        setError(t("aiProvider.errorUrlScheme"));
        return;
      }
      // The notes under the fields say what changed and why.
      if (guardKeyOrigin(baseUrl)) return;
      if (syncEndpointSettings(baseUrl)) return;
      const nextKind = applyUrlPosture(url);
      if (!editing && keyRequired && !apiKey.trim()) {
        setError(t("aiProvider.errorApiKeyRequired", { label: preset?.label ?? "" }));
        return;
      }
      setStep(nextKind !== "local" ? "privacy" : "check");
      return;
    }
    if (step === "privacy") {
      if (ackNeededFor(kind) && !remoteAcknowledged) {
        setError(t("aiProvider.errorAckRequiredNext"));
        return;
      }
      setStep("check");
    }
  };

  const goBack = () => {
    setError(null);
    const previous = stepIds[stepIndex - 1];
    if (previous) setStep(previous);
  };

  const routePresets = route ? presetsForRoute(route) : [];

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="flex max-h-[min(90vh,46rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl">
        <DialogHeader className="gap-3 border-b px-6 pt-6 pb-4">
          <div className="grid gap-1.5">
            <DialogTitle className="flex items-center gap-2">
              <Sparkles className="size-4 text-primary" aria-hidden="true" />
              {editing ? t("aiProvider.editTitle") : t("aiProvider.addTitle")}
            </DialogTitle>
            <DialogDescription>{t("aiProvider.wizardDescription")}</DialogDescription>
          </div>
          <OnboardingStepper
            steps={stepIds.map((id) => ({
              label: t(`aiProvider.step.${id}`),
              summary: stepSummaries[id],
            }))}
            current={stepIndex}
            onJump={(index) => {
              setError(null);
              const target = stepIds[index];
              if (target) setStep(target);
            }}
          />
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!editing && !lastStep) {
              goNext();
              return;
            }
            void handleSubmit(event);
          }}
          className="flex min-h-0 flex-1 flex-col"
          noValidate
        >
          <div className="min-h-80 flex-1 overflow-y-auto px-6 py-5">
            {/* Every step stays mounted: what was entered on one is kept
                while another is in view. */}
            <div className="grid gap-4" data-wizard-step="provider" hidden={step !== "provider"}>
              <div
                role="radiogroup"
                aria-label={t("aiProvider.routeLabel")}
                className="grid gap-2"
              >
                {AI_PROVIDER_ROUTES.map(({ id }) => {
                  const Icon = ROUTE_ICONS[id];
                  const checked = route === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      role="radio"
                      aria-checked={checked}
                      data-provider-route={id}
                      onClick={() => {
                        setRoute(id);
                        if (preset && routeForPreset(preset.id) !== id) setPreset(null);
                      }}
                      className={cn(
                        "flex w-full items-start gap-3 rounded-lg bg-card px-3 py-3 text-left outline-none ring-1 ring-border transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring",
                        checked && "bg-primary/5 ring-2 ring-primary hover:bg-primary/5",
                      )}
                    >
                      <span className="grid size-8 shrink-0 place-items-center rounded-md bg-muted text-foreground">
                        <Icon className="size-4" aria-hidden="true" />
                      </span>
                      <span className="grid min-w-0 flex-1 gap-0.5">
                        <span className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-medium text-foreground">
                            {t(`aiProvider.route.${id}.title`)}
                          </span>
                          <PostureBadge
                            posture={id === "local" ? "local" : "remote"}
                            title={t(`aiProvider.route.${id}.reach`)}
                          />
                        </span>
                        <span className="text-xs text-muted-foreground">
                          {t(`aiProvider.route.${id}.description`)}
                        </span>
                      </span>
                      <span
                        aria-hidden="true"
                        className={cn(
                          "mt-1 grid size-4 shrink-0 place-items-center rounded-full ring-1 ring-border",
                          checked && "bg-primary text-primary-foreground ring-primary",
                        )}
                      >
                        {checked ? <Check className="size-3" /> : null}
                      </span>
                    </button>
                  );
                })}
              </div>
              {route ? (
                <div className="grid gap-2">
                  <p id="ai-form-presets" className="text-sm font-medium">
                    {t(`aiProvider.route.${route}.which`)}
                  </p>
                  <div
                    role="radiogroup"
                    aria-labelledby="ai-form-presets"
                    className="grid grid-cols-1 gap-2 sm:grid-cols-2"
                  >
                    {routePresets.map((option) => {
                      const checked = preset?.id === option.id;
                      const label = option.custom ? t(customPresetLabelKey(option)) : option.label;
                      return (
                        <button
                          key={option.id}
                          type="button"
                          role="radio"
                          aria-checked={checked}
                          data-provider-preset={option.id}
                          onClick={() => applyPreset(option)}
                          className={cn(
                            "flex min-w-0 items-center gap-2.5 rounded-lg bg-card px-3 py-2.5 text-left text-sm outline-none ring-1 ring-border transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring",
                            checked && "bg-primary/5 ring-2 ring-primary hover:bg-primary/5",
                          )}
                        >
                          {option.custom ? (
                            <span className="grid size-5 shrink-0 place-items-center text-muted-foreground">
                              <Plug className="size-4" aria-hidden="true" />
                            </span>
                          ) : (
                            <ProviderGlyph
                              provider={{
                                name: option.display_name,
                                display_name: option.display_name,
                                base_url: option.base_url,
                              }}
                              className="size-5"
                            />
                          )}
                          <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
                          {checked ? (
                            <Check className="size-3.5 shrink-0 text-primary" aria-hidden="true" />
                          ) : null}
                        </button>
                      );
                    })}
                  </div>
                  {preset ? (
                    <p className="text-xs text-muted-foreground" data-preset-hint>
                      {t(preset.hintKey, { label: preset.label })}
                    </p>
                  ) : null}
                </div>
              ) : null}
            </div>

            <div className="grid gap-4" data-wizard-step="connect" hidden={step !== "connect"}>
              <div className="grid gap-2">
                <Label htmlFor="ai-form-name">{t("aiProvider.nameLabel")}</Label>
                <Input
                  id="ai-form-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="oMLX"
                  aria-describedby="ai-form-name-hint"
                />
                <p id="ai-form-name-hint" className="text-xs text-muted-foreground">
                  {t("aiProvider.nameHint")}
                </p>
              </div>
              <div className="grid gap-2">
                <Label htmlFor="ai-form-url">{t("aiProvider.urlLabel")}</Label>
                <Input
                  id="ai-form-url"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  onBlur={() => {
                    applyUrlPosture(baseUrl);
                    guardKeyOrigin(baseUrl);
                    syncEndpointSettings(baseUrl);
                  }}
                  placeholder="http://127.0.0.1:8000/v1 or claude-cli://default"
                />
                {autoRemote && autoRemoteShown ? (
                  <p
                    role="status"
                    data-auto-remote={autoRemote}
                    className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300"
                  >
                    <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                    <span>{t(AUTO_REMOTE_NOTE_KEYS[autoRemote])}</span>
                  </p>
                ) : null}
                {endpointReset ? (
                  <p
                    data-endpoint-settings-reset={endpointReset}
                    className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300"
                  >
                    <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                    <span>
                      {t(
                        endpointReset === "exemption"
                          ? "aiProvider.exemptionResetForAddress"
                          : "aiProvider.formatResetForAddress",
                      )}
                    </span>
                  </p>
                ) : null}
              </div>
              <div className="grid gap-2">
                <Label htmlFor="ai-form-key">
                  {t("aiProvider.apiKeyLabel")}{" "}
                  <span className="text-xs text-muted-foreground">
                    {keyRequired
                      ? t("aiProvider.apiKeyRequiredHint")
                      : kind === "local"
                        ? t("aiProvider.apiKeyLocalHint")
                        : t("aiProvider.apiKeyRemoteHint")}
                  </span>
                </Label>
                <Input
                  id="ai-form-key"
                  type="password"
                  autoComplete="off"
                  value={apiKey}
                  aria-required={keyRequired && !editing ? true : undefined}
                  aria-describedby="ai-form-key-hint"
                  onChange={(event) => {
                    setApiKey(event.target.value);
                    setKeyOrigin(httpOrigin(baseUrl));
                    setKeyCleared(false);
                  }}
                  placeholder={
                    editing && initial?.has_api_key
                      ? t("aiProvider.apiKeyPlaceholderKeep")
                      : "sk-…"
                  }
                />
                <p
                  id="ai-form-key-hint"
                  className="flex items-start gap-1.5 text-xs text-muted-foreground"
                >
                  <KeyRound className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                  <span>{t("aiProvider.apiKeyStorageHint")}</span>
                </p>
                {keyCleared ? (
                  <p
                    data-key-cleared
                    className="flex items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300"
                  >
                    <Info className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                    <span>{t("aiProvider.keyClearedForAddress")}</span>
                  </p>
                ) : null}
              </div>
              <details
                className="group rounded-md border"
                open={advancedOpen}
                onToggle={(event) => setAdvancedOpen(event.currentTarget.open)}
                data-provider-advanced
              >
                <summary className="flex cursor-pointer list-none items-center gap-2 rounded-md px-3 py-2 text-xs transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                  <ChevronRight
                    className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
                    aria-hidden="true"
                  />
                  <span className="font-medium">{t("aiProvider.advancedLabel")}</span>
                  <span className="ml-auto truncate text-muted-foreground">
                    {isCliUrl
                      ? t(`aiProvider.postureName.${kind}`)
                      : t("aiProvider.advancedSummary", {
                          format: t(`aiProvider.apiFormat.${apiFormat}`),
                        })}
                  </span>
                </summary>
                <div className="grid gap-3 border-t px-3 py-2.5">
                  <div className="grid gap-1.5">
                    <p id="ai-form-kind" className="text-xs font-medium">
                      {t("aiProvider.postureLabel")}
                    </p>
                    <div
                      role="group"
                      aria-labelledby="ai-form-kind"
                      className="grid grid-cols-3 gap-1.5"
                    >
                      {(["local", "remote", "tee"] as const).map((option) => (
                        <button
                          key={option}
                          type="button"
                          aria-pressed={kind === option}
                          onClick={() => {
                            setKind(option);
                            setAutoRemote(null);
                          }}
                          className={cn(
                            "rounded-md border px-2 py-1.5 text-xs font-medium uppercase tracking-wide transition-colors",
                            kind === option
                              ? "border-primary bg-primary text-primary-foreground"
                              : "border-border bg-background text-muted-foreground hover:bg-muted",
                          )}
                        >
                          {t(`aiProvider.posture.${option}`)}
                        </button>
                      ))}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {t(PROVIDER_KIND_HINT_KEYS[kind])}
                    </p>
                  </div>
                  {!isCliUrl ? (
                    <>
                      <p id="ai-form-api-format" className="text-xs font-medium">
                        {t("aiProvider.apiFormatLabel")}
                      </p>
                      <div
                        role="radiogroup"
                        aria-labelledby="ai-form-api-format"
                        className="grid gap-1.5"
                      >
                        {API_FORMATS.map((option) => (
                          <label
                            key={option}
                            className={cn(
                              "flex cursor-pointer items-start gap-2 rounded-md border px-2.5 py-2 text-xs transition-colors",
                              apiFormat === option
                                ? "border-primary bg-primary/5"
                                : "border-border hover:bg-muted/40",
                            )}
                          >
                            <input
                              type="radio"
                              name="ai-form-api-format"
                              value={option}
                              checked={apiFormat === option}
                              onChange={() => {
                                syncEndpointSettings(baseUrl);
                                setEndpointReset(null);
                                setApiFormat(option);
                              }}
                              aria-labelledby={`ai-form-api-format-${option}`}
                              aria-describedby={`ai-form-api-format-${option}-hint`}
                              className="mt-0.5 accent-primary"
                            />
                            <span className="grid gap-0.5">
                              <span id={`ai-form-api-format-${option}`} className="font-medium">
                                {t(`aiProvider.apiFormat.${option}`)}
                              </span>
                              <span
                                id={`ai-form-api-format-${option}-hint`}
                                className="text-muted-foreground"
                              >
                                {t(`aiProvider.apiFormatHint.${option}`)}
                              </span>
                            </span>
                          </label>
                        ))}
                      </div>
                      {savedDetection ? (
                        <p className="text-xs text-muted-foreground">
                          {t("aiProvider.apiFormatLastDetected", {
                            format: t(`aiProvider.apiFormatName.${savedDetection}`),
                          })}
                        </p>
                      ) : null}
                      <label
                        htmlFor="ai-form-omit-store"
                        className="flex cursor-pointer items-start gap-2 rounded-md border px-2.5 py-2 text-xs"
                        data-omit-store-option
                      >
                        <Checkbox
                          id="ai-form-omit-store"
                          checked={omitStoreField}
                          onCheckedChange={(checked) => {
                            syncEndpointSettings(baseUrl);
                            setEndpointReset(null);
                            setOmitStoreField(checked === true);
                          }}
                          aria-describedby="ai-form-omit-store-hint"
                          className="mt-0.5"
                        />
                        <span className="grid gap-0.5">
                          <span className="font-medium">{t("aiProvider.omitStoreLabel")}</span>
                          <span id="ai-form-omit-store-hint" className="text-muted-foreground">
                            {t("aiProvider.omitStoreHint")}
                          </span>
                        </span>
                      </label>
                    </>
                  ) : null}
                  <div className="grid gap-1.5">
                    <Label htmlFor="ai-form-notes" className="text-xs">
                      {t("aiProvider.notesLabel")}
                    </Label>
                    <Input
                      id="ai-form-notes"
                      value={notes}
                      onChange={(event) => setNotes(event.target.value)}
                      placeholder={t("aiProvider.notesPlaceholder")}
                    />
                  </div>
                </div>
              </details>
            </div>

            <div className="grid gap-4" data-wizard-step="privacy" hidden={step !== "privacy"}>
              <div className="flex items-start gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
                <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <div className="grid gap-1.5">
                  <p className="font-medium">
                    {t(kind === "tee" ? "aiProvider.privacy.titleTee" : "aiProvider.privacy.title", {
                      name: name.trim() || stepSummaries.provider || "",
                    })}
                  </p>
                  <p className="text-xs">{t("aiProvider.remoteWarning")}</p>
                </div>
              </div>
              <ul className="grid gap-1.5 text-xs text-muted-foreground">
                <li className="flex items-start gap-2">
                  <Check className="mt-px size-3.5 shrink-0 text-emerald-600" aria-hidden="true" />
                  <span>{t("aiProvider.privacy.onlyWhenAsked")}</span>
                </li>
                <li className="flex items-start gap-2">
                  <Check className="mt-px size-3.5 shrink-0 text-emerald-600" aria-hidden="true" />
                  <span>{t("aiProvider.privacy.noSecrets")}</span>
                </li>
                <li className="flex items-start gap-2">
                  <Check className="mt-px size-3.5 shrink-0 text-emerald-600" aria-hidden="true" />
                  <span>
                    {t(isCliUrl ? "aiProvider.privacy.cliTerms" : "aiProvider.privacy.retention")}
                  </span>
                </li>
              </ul>
              {needsRemoteAck ? (
                <label
                  htmlFor="ai-form-remote-ack"
                  className="flex cursor-pointer items-start gap-2 rounded-lg border p-3 text-sm"
                >
                  <Checkbox
                    id="ai-form-remote-ack"
                    checked={remoteAcknowledged}
                    onCheckedChange={(checked) => {
                      setRemoteAcknowledged(checked === true);
                      setError(null);
                    }}
                    className="mt-0.5"
                  />
                  <span>{t("aiProvider.remoteAck")}</span>
                </label>
              ) : (
                <p className="text-xs text-muted-foreground">{t("aiProvider.privacy.acknowledged")}</p>
              )}
            </div>

            <div className="grid gap-4" data-wizard-step="check" hidden={step !== "check"}>
              <div className="grid gap-2">
                <Label htmlFor="ai-form-default-model">
                  {t("aiProvider.defaultModelLabel")}
                </Label>
                <div className="flex gap-2">
                  <Input
                    id="ai-form-default-model"
                    value={defaultModel}
                    onChange={(event) => setDefaultModel(event.target.value)}
                    placeholder="qwen3.6:35b"
                    list={modelSuggestions.length > 0 ? "ai-form-model-options" : undefined}
                    autoComplete="off"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    className="shrink-0 gap-1.5"
                    onClick={() => void handleTest()}
                    disabled={isBusy || Boolean(testUnavailable)}
                    title={testUnavailable ?? t("aiProvider.testHint")}
                  >
                    {testStatus.state === "running" ? (
                      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <Activity className="size-3.5" aria-hidden="true" />
                    )}
                    {t("aiProvider.testConnection")}
                  </Button>
                </div>
                {modelSuggestions.length > 0 ? (
                  <>
                    <datalist id="ai-form-model-options">
                      {modelSuggestions.map((id) => (
                        <option key={id} value={id} />
                      ))}
                    </datalist>
                    <p className="text-xs text-muted-foreground">
                      {t("aiProvider.modelSuggestions", { count: modelSuggestions.length })}
                    </p>
                  </>
                ) : (
                  <p className="text-xs text-muted-foreground">{t("aiProvider.checkHint")}</p>
                )}
              </div>
              {testStatus.state !== "idle" ? (
                <div
                  aria-live="polite"
                  data-test-result={testStatus.state}
                  className="grid gap-1 rounded-lg border bg-muted/20 p-3 text-xs"
                >
                  {testStatus.state === "running" ? (
                    <p className="inline-flex items-center gap-1 text-muted-foreground">
                      <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                      {t("aiProvider.testing")}
                    </p>
                  ) : null}
                  {testStatus.state === "fail" ? (
                    <p className="text-destructive [overflow-wrap:anywhere]">
                      {t("aiProvider.testFailed", { message: testStatus.message })}
                    </p>
                  ) : null}
                  {testStatus.state === "no_model" ? (
                    <>
                      <p className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300">
                        <AlertTriangle className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                        <span>
                          {t("aiProvider.testNoReply", { count: testStatus.modelCount })}
                        </span>
                      </p>
                      <p className="pl-5 text-muted-foreground">{t("aiProvider.testNoModel")}</p>
                    </>
                  ) : null}
                  {testStatus.state === "ok" && testStatus.checkKind === "binary_presence" ? (
                    <p className="text-emerald-600 dark:text-emerald-400">
                      {t("aiProvider.testOkBinary")}
                    </p>
                  ) : null}
                  {testStatus.state === "ok" && testStatus.checkKind !== "binary_presence" ? (
                    <>
                      <p className="flex items-start gap-1.5 text-emerald-600 dark:text-emerald-400">
                        <CheckCircle2 className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                        <span>
                          {t("aiProvider.testOkConnected", { count: testStatus.modelCount })}
                        </span>
                      </p>
                      <ul className="grid gap-0.5 pl-5 text-muted-foreground">
                        {testStatus.probeModel ? (
                          <li>{t("aiProvider.testReplyChecked", { model: testStatus.probeModel })}</li>
                        ) : null}
                        {testStatus.detectedApiFormat ? (
                          <li data-detected-api-format={testStatus.detectedApiFormat}>
                            {t(
                              testStatus.testedApiFormat === "auto"
                                ? "aiProvider.testFormatDetected"
                                : "aiProvider.testFormatSet",
                              {
                                format: t(`aiProvider.apiFormatName.${testStatus.detectedApiFormat}`),
                              },
                            )}
                          </li>
                        ) : null}
                        {testStatus.toolsAccepted === true ? (
                          <li>{t("aiProvider.testToolsAccepted")}</li>
                        ) : null}
                      </ul>
                      {testStatus.storeAccepted === false ? (
                        <p
                          data-store-refused
                          className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300"
                        >
                          <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                          <span>{t("aiProvider.testStoreRefused")}</span>
                        </p>
                      ) : null}
                      {testStatus.storeAccepted === null && omitStoreToSave ? (
                        <p
                          data-store-omitted
                          className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300"
                        >
                          <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                          <span>{t("aiProvider.testStoreOmitted")}</span>
                        </p>
                      ) : null}
                      {testStatus.toolsAccepted === false ? (
                        <p
                          data-tools-refused
                          className="flex items-start gap-1.5 text-amber-700 dark:text-amber-300"
                        >
                          <ShieldAlert className="mt-px size-3.5 shrink-0" aria-hidden="true" />
                          <span>{t("aiProvider.testToolsRefused")}</span>
                        </p>
                      ) : null}
                    </>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>
          <DialogFooter className="items-center gap-2 border-t px-6 py-3 sm:justify-between">
            <p role="alert" className="min-h-4 text-xs text-destructive sm:flex-1">
              {error}
            </p>
            <div className="flex gap-2">
              {stepIndex === 0 ? (
                <Button type="button" variant="ghost" onClick={onClose} disabled={isBusy}>
                  {t("common:actions.cancel")}
                </Button>
              ) : (
                <Button type="button" variant="ghost" onClick={goBack} disabled={isBusy}>
                  {t("aiProvider.back")}
                </Button>
              )}
              {editing ? (
                <>
                  {!lastStep ? (
                    <Button type="button" variant="outline" onClick={goNext}>
                      {t("aiProvider.next")}
                    </Button>
                  ) : null}
                  <Button type="submit" disabled={!canSubmit}>
                    {t("common:actions.save")}
                  </Button>
                </>
              ) : lastStep ? (
                <Button type="submit" disabled={!canSubmit}>
                  {t("aiProvider.addProviderButton")}
                </Button>
              ) : (
                <Button type="submit" disabled={step === "provider" && !preset}>
                  {t("aiProvider.next")}
                </Button>
              )}
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
