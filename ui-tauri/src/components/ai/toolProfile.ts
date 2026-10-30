/**
 * The tool sets a provider can offer the model, as its settings present them.
 * All tools is the default for every provider and model: Kassiber cannot tell
 * how capable a model is, so the user narrows the set, never the app. The set
 * follows the provider, not the chat.
 */

import type { AiToolProfile } from "@/daemon/stream";

export const TOOL_PROFILE_OPTIONS = [
  {
    value: "full",
    labelKey: "composer.toolProfile.full.label",
    descriptionKey: "composer.toolProfile.full.description",
  },
  {
    value: "scoped",
    labelKey: "composer.toolProfile.scoped.label",
    descriptionKey: "composer.toolProfile.scoped.description",
  },
  {
    value: "core",
    labelKey: "composer.toolProfile.core.label",
    descriptionKey: "composer.toolProfile.core.description",
  },
] as const satisfies ReadonlyArray<{
  value: AiToolProfile;
  labelKey: string;
  descriptionKey: string;
}>;
