/**
 * What Incognito can promise for the selected provider.
 *
 * Incognito always keeps the conversation out of Kassiber's history. Beyond
 * that it depends on the provider (docs/reference/privacy-and-security.md):
 * - a local HTTP provider is asked not to store the request; Kassiber can't
 *   control what that server itself logs;
 * - Codex, Claude and OpenCode are started without session persistence,
 *   except OpenCode on Windows, which refuses Incognito turns;
 * - GitHub Copilot cannot skip saving, so its session file lives in the
 *   turn's temporary folder until that turn ends;
 * - an off-device HTTP provider is asked not to store the request, but its
 *   own retention still applies.
 */

import { useDaemon } from "@/daemon/client";
import {
  nativeAiProviderRuntime,
  type AiProviderRow,
  type AiProvidersListData,
  type AssistantModelSelection,
} from "@/lib/aiCapabilities";

export type IncognitoScope =
  | "local"
  | "cli"
  | "copilot"
  | "opencodeWindows"
  | "remote";

function isWindowsPlatform(
  userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent,
): boolean {
  return /Windows/i.test(userAgent);
}

export function incognitoScope(
  provider: Pick<AiProviderRow, "base_url" | "kind"> | null | undefined,
  windows = isWindowsPlatform(),
): IncognitoScope | null {
  if (!provider) return null;
  const runtime = nativeAiProviderRuntime(provider.base_url);
  if (runtime === "copilot") return "copilot";
  // The broker refuses OpenCode Incognito turns on Windows; say so up front.
  if (runtime === "opencode" && windows) return "opencodeWindows";
  if (runtime !== null) return "cli";
  return provider.kind === "local" ? "local" : "remote";
}

/** The scope for the selected provider; null until the provider list loads. */
export function useIncognitoScope(
  selection: AssistantModelSelection,
): IncognitoScope | null {
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    { meta: { shellProgress: false } },
  );
  const providers =
    providersQuery.data?.kind === "ai.providers.list"
      ? (providersQuery.data.data?.providers ?? [])
      : [];
  const selected = selection
    ? providers.find((provider) => provider.name === selection.provider)
    : undefined;
  return incognitoScope(selected);
}
