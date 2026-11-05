/**
 * The chat permission modes as the composer and Settings present them: an
 * icon, a label and a one-line description each, in the order of how much the
 * assistant may do on its own. The layout follows T3 Code's runtime-mode
 * picker; the daemon decides which calls each mode covers.
 */

import { Lock, LockOpen, Sparkles, type LucideIcon } from "lucide-react";

import type { AiPermissionMode, AiToolApproval } from "@/daemon/stream";

export const PERMISSION_MODE_OPTIONS = [
  {
    value: "approval",
    icon: Lock,
    labelKey: "composer.permission.approval.label",
    descriptionKey: "composer.permission.approval.description",
  },
  {
    value: "auto",
    icon: Sparkles,
    labelKey: "composer.permission.auto.label",
    descriptionKey: "composer.permission.auto.description",
  },
  {
    value: "full_access",
    icon: LockOpen,
    labelKey: "composer.permission.full_access.label",
    descriptionKey: "composer.permission.full_access.description",
  },
] as const satisfies ReadonlyArray<{
  value: AiPermissionMode;
  icon: LucideIcon;
  labelKey: string;
  descriptionKey: string;
}>;

export type PermissionModeOption = (typeof PERMISSION_MODE_OPTIONS)[number];

export function permissionModeOption(mode: AiPermissionMode): PermissionModeOption {
  return (
    PERMISSION_MODE_OPTIONS.find((option) => option.value === mode) ??
    PERMISSION_MODE_OPTIONS[0]
  );
}

/**
 * The badge for a call that ran without asking, keyed by the daemon's
 * `approval`; null when the user approved it (or nothing needed approval).
 */
export const AUTO_APPROVAL_BADGE_KEYS = {
  auto: "tool.approval.auto",
  full_access: "tool.approval.full_access",
  session: "tool.approval.session",
} as const satisfies Partial<Record<AiToolApproval, string>>;

export function autoApprovalBadgeKey(
  approval: AiToolApproval | undefined,
): (typeof AUTO_APPROVAL_BADGE_KEYS)[keyof typeof AUTO_APPROVAL_BADGE_KEYS] | null {
  return approval && approval in AUTO_APPROVAL_BADGE_KEYS
    ? AUTO_APPROVAL_BADGE_KEYS[approval as keyof typeof AUTO_APPROVAL_BADGE_KEYS]
    : null;
}
