/**
 * The provider form's **Test connection**: what it sends to
 * `ai.test_connection` and how it reads the answer.
 *
 * For an HTTP provider the daemon lists the models and then asks one model a
 * fixed question to learn which API format answers and whether tools are
 * accepted. That needs a model, so the form sends the one it holds, and the
 * format setting it is about to save; the detected format comes back for the
 * form to save with a new or retargeted provider.
 */

import type { AiProviderInput } from "@/components/kb/AiProviderForm";
import type { AiApiFormat, AiModelRow, AiWireApiFormat } from "@/lib/aiCapabilities";

export interface AiTestConnectionData {
  check_kind?: string;
  model_count?: number;
  models?: AiModelRow[];
  /** The format setting tested; null for a CLI agent. */
  api_format?: AiApiFormat | null;
  detected_api_format?: AiWireApiFormat | null;
  tools_accepted?: boolean | null;
  /**
   * false: a Chat Completions endpoint refused `store: false` and answered
   * the fixed test prompt without it. null: not asked.
   */
  store_accepted?: boolean | null;
  probe?: {
    status?: "ok" | "skipped";
    reason?: "no_model" | "cli_provider" | CliProxyProbeSkip | null;
    model?: string | null;
    finish_reason?: string | null;
    /** CLI proxy: the format the model was asked in, from its family. */
    api_format?: AiWireApiFormat | null;
  };
  /** CLI proxy: the listing became the saved provider's model list. */
  models_saved?: boolean;
}

/** Why a CLI proxy test asked no model a question. */
export type CliProxyProbeSkip = "model_not_listed" | "not_a_chat_model" | "unknown_family";

function cliProxySkip(value: unknown): CliProxyProbeSkip | null {
  return value === "model_not_listed" || value === "not_a_chat_model" || value === "unknown_family"
    ? value
    : null;
}

export type AiProviderTestOutcome =
  | {
      state: "ok";
      modelCount: number;
      checkKind?: string;
      /** The format that answered; saved with the row. */
      detectedApiFormat: AiWireApiFormat | null;
      /** The setting tested: "auto" detects, a wire format was pinned. */
      testedApiFormat: AiApiFormat | null;
      /** false: the endpoint answered only without tools. null: not asked. */
      toolsAccepted: boolean | null;
      /** false: the endpoint refuses Kassiber's `store: false`. null: not asked. */
      storeAccepted: boolean | null;
      /** The model that answered the test prompt, when one was asked. */
      probeModel: string | null;
      /** CLI proxy: the format that model was asked in. */
      probeApiFormat: AiWireApiFormat | null;
      /** CLI proxy: the listing is now the saved provider's model list. */
      modelsSaved: boolean;
      models: AiModelRow[];
    }
  /**
   * Models listed, but nothing could ask one: generation is unchecked. For a
   * CLI proxy, `skipped` says why the chosen model was not asked.
   */
  | {
      state: "no_model";
      modelCount: number;
      models: AiModelRow[];
      skipped?: { reason: CliProxyProbeSkip; model: string };
      modelsSaved: boolean;
    };

function wireFormat(value: unknown): AiWireApiFormat | null {
  return value === "responses" || value === "chat_completions" ? value : null;
}

export function aiTestConnectionArgs({
  baseUrl,
  kind,
  apiKey,
  model,
  provider,
  apiFormat,
  omitStoreField,
  acknowledged,
}: {
  baseUrl: string;
  kind: AiProviderInput["kind"];
  apiKey: string;
  /** The form's default model, which starts as the saved one. */
  model: string;
  /** The saved provider being edited; its key is reused when none is typed. */
  provider?: string | null;
  /** The format setting the form would save; omitted for a CLI agent. */
  apiFormat?: AiApiFormat | null;
  /** The form's store choice; omitted for a CLI agent, which has none. */
  omitStoreField?: boolean | null;
  /**
   * The form's off-device acknowledgement, ticked: the daemon refuses to test
   * an off-device target without it or a saved provider's own.
   */
  acknowledged?: boolean;
}): Record<string, unknown> {
  // The kind decides whether the test may run in offline mode: only a local
  // provider keeps working there.
  const args: Record<string, unknown> = { base_url: baseUrl.trim(), kind };
  if (apiKey.trim()) args.api_key = apiKey.trim();
  if (model.trim()) args.model = model.trim();
  if (provider) args.provider = provider;
  if (apiFormat) args.api_format = apiFormat;
  if (typeof omitStoreField === "boolean") args.omit_store_field = omitStoreField;
  if (acknowledged) args.acknowledged = true;
  return args;
}

export function aiProviderTestOutcome(
  data: AiTestConnectionData | undefined,
): AiProviderTestOutcome {
  const modelCount = data?.model_count ?? 0;
  const models = Array.isArray(data?.models) ? data.models : [];
  const modelsSaved = data?.models_saved === true;
  if (data?.probe?.status === "skipped" && data.probe.reason === "no_model") {
    return { state: "no_model", modelCount, models, modelsSaved };
  }
  const skip = data?.probe?.status === "skipped" ? cliProxySkip(data.probe.reason) : null;
  if (skip) {
    return {
      state: "no_model",
      modelCount,
      models,
      skipped: { reason: skip, model: data?.probe?.model ?? "" },
      modelsSaved,
    };
  }
  const tested = data?.api_format;
  const probeModel = data?.probe?.status === "ok" ? data.probe.model : null;
  return {
    state: "ok",
    modelCount,
    checkKind: data?.check_kind,
    detectedApiFormat: wireFormat(data?.detected_api_format),
    testedApiFormat: tested === "auto" ? "auto" : wireFormat(tested),
    toolsAccepted: typeof data?.tools_accepted === "boolean" ? data.tools_accepted : null,
    storeAccepted: typeof data?.store_accepted === "boolean" ? data.store_accepted : null,
    probeModel: typeof probeModel === "string" && probeModel ? probeModel : null,
    probeApiFormat: data?.probe?.status === "ok" ? wireFormat(data.probe.api_format) : null,
    modelsSaved,
    models,
  };
}
