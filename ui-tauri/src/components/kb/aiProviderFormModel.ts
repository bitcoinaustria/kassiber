/**
 * Pure rules behind the AI provider form (AiProviderForm.tsx), kept apart so
 * they are testable without mounting the dialog.
 */

import {
  isLoopbackAiHttpUrl,
  isNativeAiProviderLocator,
  type AiProviderKind,
} from "@/lib/aiCapabilities";

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
