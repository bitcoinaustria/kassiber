/**
 * Display helpers for Settings → AI providers. Pure functions so the status
 * language is testable without mounting the page.
 *
 * Layout and status vocabulary follow T3 Code's provider settings cards
 * (MIT, Copyright (c) 2026 T3 Tools Inc.: `providerStatus.ts`), reduced to
 * the states Kassiber's provider broker reports.
 */

import {
  NATIVE_RUNTIME_FAST_MODELS,
  NATIVE_RUNTIME_REASONING_EFFORTS,
  PROVIDER_DEFAULT_EFFORTS,
  nativeAiProviderRuntime,
  type AiModelRow,
  type AiProviderRow,
  type AiProviderRuntimeStatus,
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
 * known level. The stored value is always kept so it can be seen and cleared.
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
  const options = PROVIDER_DEFAULT_EFFORTS.filter(
    (effort) => advertised.size === 0 || advertised.has(effort) || effort === stored,
  );
  return options.length > 0 ? options : [...PROVIDER_DEFAULT_EFFORTS];
}

/**
 * Whether any model this provider is known to serve offers fast mode: the
 * broker's fixed Claude Opus entry, or a checked model that advertises it.
 */
export function providerOffersFastMode(
  provider: AiProviderRow,
  models: AiModelRow[],
): boolean {
  const runtime = nativeAiProviderRuntime(provider.base_url);
  if (!runtime) return false;
  if ((NATIVE_RUNTIME_FAST_MODELS[runtime]?.length ?? 0) > 0) return true;
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
