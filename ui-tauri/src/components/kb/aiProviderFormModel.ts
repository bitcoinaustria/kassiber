/**
 * Pure rules behind the AI provider form (AiProviderForm.tsx), kept apart so
 * they are testable without mounting the dialog.
 */

import {
  isLoopbackAiHttpUrl,
  isNativeAiProviderLocator,
  type AiProviderKind,
} from "@/lib/aiCapabilities";

type PresetHintKey =
  | "aiProvider.presetHint.ollama"
  | "aiProvider.presetHint.omlx"
  | "aiProvider.presetHint.openai"
  | "aiProvider.presetHint.openrouter"
  | "aiProvider.presetHint.custom"
  | "aiProvider.presetHint.cli";

export interface AiProviderPreset {
  /** Stable id for tests and state; never shown. */
  id: string;
  /** Brand name, shown as is in every language. */
  label: string;
  /** What the Name field starts with. */
  display_name: string;
  base_url: string;
  kind: AiProviderKind;
  default_model: string;
  /** One line under the presets saying what this one is for. */
  hintKey: PresetHintKey;
  /** The endpoint takes no request without an API key. */
  apiKeyRequired?: boolean;
  /**
   * Any other OpenAI-compatible server: the user enters the URL, and the
   * posture follows it (see `autoRemoteReason`).
   */
  custom?: boolean;
}

/**
 * Starting points for a new provider. No model lists: a hosted endpoint's
 * models change too often, so Test connection lists what it serves today.
 */
export const AI_PROVIDER_PRESETS: readonly AiProviderPreset[] = [
  {
    id: "ollama",
    label: "Ollama",
    display_name: "Ollama",
    base_url: "http://localhost:11434/v1",
    kind: "local",
    default_model: "qwen3.6:35b",
    hintKey: "aiProvider.presetHint.ollama",
  },
  {
    id: "omlx",
    label: "oMLX",
    display_name: "oMLX",
    base_url: "http://127.0.0.1:8000/v1",
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.omlx",
  },
  {
    id: "openai",
    label: "OpenAI",
    display_name: "OpenAI",
    base_url: "https://api.openai.com/v1",
    kind: "remote",
    default_model: "",
    hintKey: "aiProvider.presetHint.openai",
    apiKeyRequired: true,
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    display_name: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    kind: "remote",
    default_model: "",
    hintKey: "aiProvider.presetHint.openrouter",
    apiKeyRequired: true,
  },
  {
    id: "openai-compatible",
    label: "OpenAI-compatible",
    display_name: "",
    base_url: "",
    // Until a URL says otherwise; an address off this device moves it.
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.custom",
    custom: true,
  },
  {
    id: "claude-cli",
    label: "Claude CLI",
    display_name: "Claude CLI",
    base_url: "claude-cli://default",
    kind: "remote",
    default_model: "default",
    hintKey: "aiProvider.presetHint.cli",
  },
  {
    id: "codex-cli",
    label: "Codex CLI",
    display_name: "Codex CLI",
    base_url: "codex-cli://default",
    kind: "remote",
    default_model: "default",
    hintKey: "aiProvider.presetHint.cli",
  },
  {
    id: "opencode-cli",
    label: "OpenCode CLI",
    display_name: "OpenCode CLI",
    base_url: "opencode-cli://default",
    kind: "remote",
    default_model: "",
    hintKey: "aiProvider.presetHint.cli",
  },
  {
    id: "copilot-cli",
    label: "GitHub Copilot CLI",
    display_name: "GitHub Copilot CLI",
    base_url: "copilot-cli://default",
    kind: "remote",
    default_model: "default",
    hintKey: "aiProvider.presetHint.cli",
  },
];

/** Whether the form must have a key: a keyed preset still at its own URL. */
export function presetNeedsApiKey(preset: AiProviderPreset | null, baseUrl: string): boolean {
  return Boolean(preset?.apiKeyRequired && baseUrl.trim() === preset.base_url);
}

/** Why the form moved a provider from local to remote on its own. */
export type AutoRemoteReason = "url" | "cli";

/**
 * Whether the form must move a provider to remote for `baseUrl`, and why.
 *
 * Only ever local to remote, as the daemon infers it: an HTTP address off this
 * device cannot be local, and neither can a CLI agent, which may send prompts
 * to its model service. Remote and TEE are the user's own claims and stay, and
 * nothing is ever moved to local. Null while the URL has no host to judge.
 */
export function autoRemoteReason(
  kind: AiProviderKind,
  baseUrl: string,
): AutoRemoteReason | null {
  if (kind !== "local") return null;
  const url = baseUrl.trim();
  if (isNativeAiProviderLocator(url)) return "cli";
  if (!/^https?:\/\/[^/?#]/.test(url)) return null;
  return isLoopbackAiHttpUrl(url) ? null : "url";
}
