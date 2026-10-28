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
  | "aiProvider.presetHint.lmstudio"
  | "aiProvider.presetHint.vllm"
  | "aiProvider.presetHint.llamacpp"
  | "aiProvider.presetHint.openai"
  | "aiProvider.presetHint.openrouter"
  | "aiProvider.presetHint.custom"
  | "aiProvider.presetHint.customApi"
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
  /**
   * The command that starts the server the way Kassiber needs it, shown
   * under the hint to copy. Never run by Kassiber.
   */
  command?: string;
  /** The endpoint takes no request without an API key. */
  apiKeyRequired?: boolean;
  /**
   * Any other OpenAI-compatible server: the user enters the URL, and the
   * posture follows it (see `autoRemoteReason`).
   */
  custom?: boolean;
}

/**
 * How a provider is reached, which is also where prompts go: a server on
 * this device, the vendor app the user is signed in to (its CLI), or a
 * hosted API with a key. The add wizard's first step chooses one; each
 * offers its presets, and its own "other" for any OpenAI-compatible server.
 */
export type AiProviderRoute = "local" | "app" | "api";

export const AI_PROVIDER_ROUTES: readonly { id: AiProviderRoute; presets: readonly string[] }[] = [
  {
    id: "local",
    presets: ["ollama", "lmstudio", "vllm", "llamacpp", "omlx", "openai-compatible"],
  },
  { id: "app", presets: ["claude-cli", "codex-cli", "opencode-cli", "copilot-cli"] },
  { id: "api", presets: ["openai", "openrouter", "openai-compatible-api"] },
];

/** The route a preset belongs to. */
export function routeForPreset(presetId: string): AiProviderRoute | null {
  return AI_PROVIDER_ROUTES.find((route) => route.presets.includes(presetId))?.id ?? null;
}

export function presetsForRoute(route: AiProviderRoute): AiProviderPreset[] {
  const ids = AI_PROVIDER_ROUTES.find((entry) => entry.id === route)?.presets ?? [];
  return ids.flatMap((id) => AI_PROVIDER_PRESETS.filter((preset) => preset.id === id));
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
    command: "ollama serve",
  },
  {
    id: "lmstudio",
    label: "LM Studio",
    display_name: "LM Studio",
    base_url: "http://localhost:1234/v1",
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.lmstudio",
    command: "lms server start",
  },
  {
    id: "vllm",
    label: "vLLM",
    display_name: "vLLM",
    base_url: "http://127.0.0.1:8000/v1",
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.vllm",
    // Without a tool parser vLLM refuses tools, and the assistant cannot
    // read the book. The parser depends on the model.
    command: "vllm serve <model> --enable-auto-tool-choice --tool-call-parser hermes",
  },
  {
    id: "llamacpp",
    label: "llama.cpp",
    display_name: "llama.cpp",
    base_url: "http://127.0.0.1:8080/v1",
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.llamacpp",
    // Tool calls need the model's chat template, which --jinja turns on.
    command: "llama-server -m <model.gguf> --jinja",
  },
  {
    id: "omlx",
    label: "oMLX",
    display_name: "oMLX",
    base_url: "http://127.0.0.1:8000/v1",
    kind: "local",
    default_model: "",
    hintKey: "aiProvider.presetHint.omlx",
    command: "omlx start",
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
    id: "openai-compatible-api",
    label: "OpenAI-compatible API",
    display_name: "",
    base_url: "",
    // A hosted API sends prompts off this device, whatever its address.
    kind: "remote",
    default_model: "",
    hintKey: "aiProvider.presetHint.customApi",
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

/**
 * The origin (scheme, host and port) an HTTP base URL points at, lowercased,
 * or null for anything else, a CLI locator included. An API key typed into
 * the form belongs to the origin it was typed for.
 */
export function httpOrigin(raw: string): string | null {
  let parsed: URL;
  try {
    parsed = new URL(raw.trim());
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  if (!parsed.host) return null;
  return `${parsed.protocol}//${parsed.host}`.toLowerCase();
}
