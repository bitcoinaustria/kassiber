/**
 * The tool sets a chat can offer the model, as the composer and the provider
 * settings present them. All tools is the default for every provider and
 * model: Kassiber cannot tell how capable a model is, so the user narrows the
 * set, never the app.
 */

import { Box, Boxes, Focus, type LucideIcon } from "lucide-react";

import type { AiToolProfile } from "@/daemon/stream";

export const TOOL_PROFILE_OPTIONS = [
  {
    value: "full",
    icon: Boxes,
    labelKey: "composer.toolProfile.full.label",
    descriptionKey: "composer.toolProfile.full.description",
  },
  {
    value: "scoped",
    icon: Focus,
    labelKey: "composer.toolProfile.scoped.label",
    descriptionKey: "composer.toolProfile.scoped.description",
  },
  {
    value: "core",
    icon: Box,
    labelKey: "composer.toolProfile.core.label",
    descriptionKey: "composer.toolProfile.core.description",
  },
] as const satisfies ReadonlyArray<{
  value: AiToolProfile;
  icon: LucideIcon;
  labelKey: string;
  descriptionKey: string;
}>;

export function toolProfileOption(profile: AiToolProfile) {
  return (
    TOOL_PROFILE_OPTIONS.find((option) => option.value === profile) ??
    TOOL_PROFILE_OPTIONS[0]
  );
}
