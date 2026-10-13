/**
 * The provider form's **Test connection**: what it sends to
 * `ai.test_connection` and how it reads the answer.
 *
 * For an HTTP provider the daemon lists the models and then asks one model a
 * fixed question to learn which API format answers. That needs a model, so
 * the form sends the one it holds; the detected format comes back for the
 * form to save with a new or retargeted provider.
 */

import type { AiProviderInput } from "@/components/kb/AiProviderForm";
import type { AiWireApiFormat } from "@/lib/aiCapabilities";

export interface AiTestConnectionData {
  check_kind?: string;
  model_count?: number;
  detected_api_format?: AiWireApiFormat | null;
  tools_accepted?: boolean | null;
  probe?: {
    status?: "ok" | "skipped";
    reason?: "no_model" | "cli_provider" | null;
    model?: string | null;
  };
}

export type AiProviderTestOutcome =
  | {
      state: "ok";
      modelCount: number;
      checkKind?: string;
      detectedApiFormat: AiWireApiFormat | null;
    }
  /** Models listed, but nothing could ask one: generation is unchecked. */
  | { state: "no_model"; modelCount: number };

export function aiTestConnectionArgs({
  baseUrl,
  kind,
  apiKey,
  model,
  provider,
}: {
  baseUrl: string;
  kind: AiProviderInput["kind"];
  apiKey: string;
  /** The form's default model, which starts as the saved one. */
  model: string;
  /** The saved provider being edited; its key is reused when none is typed. */
  provider?: string | null;
}): Record<string, unknown> {
  // The kind decides whether the test may run in offline mode: only a local
  // provider keeps working there.
  const args: Record<string, unknown> = { base_url: baseUrl.trim(), kind };
  if (apiKey.trim()) args.api_key = apiKey.trim();
  if (model.trim()) args.model = model.trim();
  if (provider) args.provider = provider;
  return args;
}

export function aiProviderTestOutcome(
  data: AiTestConnectionData | undefined,
): AiProviderTestOutcome {
  const modelCount = data?.model_count ?? 0;
  if (data?.probe?.status === "skipped" && data.probe.reason === "no_model") {
    return { state: "no_model", modelCount };
  }
  const detected = data?.detected_api_format;
  return {
    state: "ok",
    modelCount,
    checkKind: data?.check_kind,
    detectedApiFormat:
      detected === "responses" || detected === "chat_completions" ? detected : null,
  };
}
