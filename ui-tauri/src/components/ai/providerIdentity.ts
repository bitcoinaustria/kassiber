import {
  isNativeAiProviderLocator,
  nativeAiProviderRuntime,
  type AiProviderRow,
} from "@/lib/aiCapabilities";

export function providerDisplayName(provider: AiProviderRow): string {
  return provider.display_name?.trim() || provider.name;
}

export function isCliProvider(provider: AiProviderRow): boolean {
  return isNativeAiProviderLocator(provider.base_url);
}

/**
 * The runtime id used to look up a provider's brand mark (`codex`, `claude`,
 * `opencode`, `gemini`, `copilot`). Purely cosmetic — it never decides
 * routing, posture, or whether a provider may be contacted.
 */
export function providerIconKey(provider: AiProviderRow): string | null {
  return nativeAiProviderRuntime(provider.base_url);
}

// Generic suffixes that would make "Gemini CLI" and "GitHub Copilot" both "GC".
const GENERIC_NAME_WORDS = new Set(["cli", "ai", "acp", "agent"]);

/**
 * Up to two initials for a provider without a brand mark. Adapted from T3 Code
 * (MIT, Copyright (c) 2026 T3 Tools Inc.), `providerInstanceInitials`.
 */
export function providerInitials(label: string): string {
  const allWords = label
    .replace(/[_-]+/g, " ")
    .split(/\s+/u)
    .filter(Boolean);
  const distinctive = allWords.filter(
    (word) => !GENERIC_NAME_WORDS.has(word.toLowerCase()),
  );
  const words = distinctive.length > 0 ? distinctive : allWords;
  if (words.length === 0) return "?";
  if (words.length === 1) {
    return Array.from(words[0]).slice(0, 2).join("").toUpperCase();
  }
  return words
    .slice(0, 2)
    .map((word) => Array.from(word)[0]?.toUpperCase() ?? "")
    .join("");
}
