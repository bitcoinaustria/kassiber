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
  probe?: {
    status?: "ok" | "skipped";
    reason?: "no_model" | "cli_provider" | null;
    model?: string | null;
    finish_reason?: string | null;
  };
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
      /** The model that answered the test prompt, when one was asked. */
      probeModel: string | null;
      models: AiModelRow[];
    }
  /** Models listed, but nothing could ask one: generation is unchecked. */
  | { state: "no_model"; modelCount: number; models: AiModelRow[] };

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
}): Record<string, unknown> {
  // The kind decides whether the test may run in offline mode: only a local
  // provider keeps working there.
  const args: Record<string, unknown> = { base_url: baseUrl.trim(), kind };
  if (apiKey.trim()) args.api_key = apiKey.trim();
  if (model.trim()) args.model = model.trim();
  if (provider) args.provider = provider;
  if (apiFormat) args.api_format = apiFormat;
  return args;
}

export function aiProviderTestOutcome(
  data: AiTestConnectionData | undefined,
): AiProviderTestOutcome {
  const modelCount = data?.model_count ?? 0;
  const models = Array.isArray(data?.models) ? data.models : [];
  if (data?.probe?.status === "skipped" && data.probe.reason === "no_model") {
    return { state: "no_model", modelCount, models };
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
    probeModel: typeof probeModel === "string" && probeModel ? probeModel : null,
    models,
  };
}
