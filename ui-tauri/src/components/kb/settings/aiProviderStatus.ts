/**
 * Display helpers for Settings → Assistant. Pure functions so the status
 * language is testable without mounting the page.
 *
 * Layout and status vocabulary follow T3 Code's provider settings cards
 * (MIT, Copyright (c) 2026 T3 Tools Inc.: `providerStatus.ts`), reduced to
 * the states Kassiber's provider broker reports.
 */

import {
  HTTP_REASONING_EFFORTS,
  NATIVE_RUNTIME_REASONING_EFFORTS,
  PROVIDER_DEFAULT_EFFORTS,
  nativeAiProviderRuntime,
  type AiModelRow,
  type AiProviderRow,
  type AiProviderRuntimeStatus,
  type AiWireApiFormat,
  type ProviderDefaultEffort,
} from "@/lib/aiCapabilities";

export type ProviderStatusTone = "ready" | "attention" | "error" | "muted";

export interface ProviderRuntimeSummary {
  tone: ProviderStatusTone;
  /** `settings:ai.status.*` key for the headline. */
  headlineKey:
    | "notChecked"
    | "ready"
    | "notInstalled"
    | "signInNeeded"
    | "error";
  version: string | null;
  /** Broker-sanitized hint, e.g. "Run `copilot login` outside Kassiber." */
  detail: string | null;
}

/**
 * A compact version label. CLIs answer `--version` in their own words
 * ("2.1.287 (Claude Code)", "GitHub Copilot CLI 1.0.93."); keep the version
 * number with a `v` prefix, and other tags as they are.
 */
export function providerVersionLabel(version: string | null | undefined): string | null {
  const trimmed = version?.trim();
  if (!trimmed) return null;
  const match = /\d+(?:\.\d+)+(?:-[0-9A-Za-z.]+)?/.exec(trimmed);
  if (match) return `v${match[0].replace(/\.$/, "")}`;
  return trimmed.length > 24 ? `${trimmed.slice(0, 23)}…` : trimmed;
}

export function runtimeSummary(
  status: AiProviderRuntimeStatus | null | undefined,
): ProviderRuntimeSummary {
  if (!status) {
    return { tone: "muted", headlineKey: "notChecked", version: null, detail: null };
  }
  const version = providerVersionLabel(status.version);
  const message = status.message?.trim() || null;
  switch (status.state) {
    case "ready":
      return { tone: "ready", headlineKey: "ready", version, detail: null };
    case "missing_executable":
      return { tone: "error", headlineKey: "notInstalled", version: null, detail: message };
    case "authentication_required":
      return { tone: "attention", headlineKey: "signInNeeded", version, detail: message };
    default:
      return { tone: "error", headlineKey: "error", version, detail: message };
  }
}

/** Split a hint on backticks so commands can render as inline code. */
export function hintSegments(text: string): Array<{ code: boolean; text: string }> {
  return text
    .split("`")
    .map((part, index) => ({ code: index % 2 === 1, text: part }))
    .filter((segment) => segment.text.length > 0);
}

/**
 * Levels offered for a provider's default: the CLI runtime's fixed set where
 * the broker publishes one, the levels its checked models advertise, or every
 * known level. An HTTP endpoint takes at most low, medium and high, so those
 * bound an HTTP provider's levels. The stored value is always kept so it can
 * be seen and cleared.
 */
export function providerEffortOptions(
  provider: AiProviderRow,
  models: AiModelRow[],
): ProviderDefaultEffort[] {
  const runtime = nativeAiProviderRuntime(provider.base_url);
  const fixed = runtime ? NATIVE_RUNTIME_REASONING_EFFORTS[runtime] : undefined;
  const advertised = new Set<string>(
    fixed ??
      models.flatMap((model) =>
        Array.isArray(model.reasoning_efforts)
          ? model.reasoning_efforts.filter(
              (effort): effort is string => typeof effort === "string",
            ).map((effort) => effort.toLowerCase())
          : [],
      ),
  );
  const stored = provider.default_reasoning_effort;
  const ceiling: readonly string[] | null = runtime ? null : HTTP_REASONING_EFFORTS;
  const options = PROVIDER_DEFAULT_EFFORTS.filter(
    (effort) =>
      effort === stored ||
      ((advertised.size === 0 || advertised.has(effort)) &&
        (ceiling === null || ceiling.includes(effort))),
  );
  return options.length > 0 ? options : [...PROVIDER_DEFAULT_EFFORTS];
}

export type ProviderApiFormatSummary =
  /** Pinned in the provider form. */
  | { source: "set"; format: AiWireApiFormat }
  /** Auto, and the endpoint was seen to answer this format. */
  | { source: "detected"; format: AiWireApiFormat }
  /** Auto with nothing detected: Responses first, then Chat Completions once. */
  | { source: "auto"; format: null };

function wireFormat(value: unknown): AiWireApiFormat | null {
  return value === "responses" || value === "chat_completions" ? value : null;
}

/**
 * The API format an HTTP provider's next chat speaks, and where that comes
 * from. Null for a CLI agent, which has its own protocol, and for a row from
 * a daemon that reports no format.
 */
export function providerApiFormatSummary(
  provider: AiProviderRow,
): ProviderApiFormatSummary | null {
  if (nativeAiProviderRuntime(provider.base_url)) return null;
  const pinned = wireFormat(provider.api_format);
  if (pinned) return { source: "set", format: pinned };
  const detected = wireFormat(provider.detected_api_format);
  if (detected) return { source: "detected", format: detected };
  return provider.api_format === "auto" ? { source: "auto", format: null } : null;
}

/**
 * Whether a checked model of this provider advertises fast mode. Only a check
 * knows: the broker gates it on the installed CLI's version.
 */
export function providerOffersFastMode(
  provider: AiProviderRow,
  models: AiModelRow[],
): boolean {
  if (!nativeAiProviderRuntime(provider.base_url)) return false;
  return models.some((model) => model.supports_fast_mode === true);
}

/** "3 minutes ago" in the UI language, for a check's `checked_at`. */
export function checkedAgo(
  checkedAt: string | null | undefined,
  language: string,
  now: number = Date.now(),
): string | null {
  if (!checkedAt) return null;
  const at = Date.parse(checkedAt);
  if (Number.isNaN(at)) return null;
  const seconds = Math.round((at - now) / 1000);
  const format = new Intl.RelativeTimeFormat(language, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 45) return format.format(0, "second");
  if (abs < 45 * 60) return format.format(Math.round(seconds / 60), "minute");
  if (abs < 22 * 3600) return format.format(Math.round(seconds / 3600), "hour");
  return format.format(Math.round(seconds / 86400), "day");
}
