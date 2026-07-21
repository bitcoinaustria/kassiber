import { useState } from "react";

import { Button } from "@/components/ui/button";

/**
 * The one lean confirm pattern shared by every decision card: a divider, the
 * consequences in words, cautions as plain amber sentences (no nested tinted
 * boxes), the explicit-review checkbox when the contract requires one, and
 * Confirm/Back. Visual weight stays on the card's question, not the plumbing.
 */
export function ConfirmSection({
  heading,
  lines,
  notes,
  checkboxLabel,
  confirmLabel,
  pendingLabel,
  isPending,
  disabled,
  onConfirm,
  onBack,
  backLabel,
}: {
  heading: string;
  /** Consequence sentences, rendered as a plain list. */
  lines: string[];
  /** Cautions (plan warnings, filed-report impacts) — amber text, no boxes. */
  notes?: string[];
  /** When set, confirm stays disabled until the checkbox is ticked. */
  checkboxLabel?: string;
  confirmLabel: string;
  pendingLabel: string;
  isPending: boolean;
  disabled?: boolean;
  onConfirm: () => void;
  onBack: () => void;
  backLabel: string;
}) {
  const [confirmed, setConfirmed] = useState(false);
  const blocked = Boolean(checkboxLabel) && !confirmed;
  return (
    <div className="space-y-3 border-t pt-4">
      <p className="text-sm font-medium">{heading}</p>
      <ul className="space-y-1 text-sm text-muted-foreground">
        {lines.map((line) => (
          <li key={line} className="flex gap-2">
            <span aria-hidden="true">→</span>
            <span>{line}</span>
          </li>
        ))}
      </ul>
      {notes?.length ? (
        <ul className="space-y-1 text-sm text-amber-700 dark:text-amber-300">
          {notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      ) : null}
      {checkboxLabel ? (
        <label className="flex items-start gap-2 text-sm">
          <input
            type="checkbox"
            className="mt-0.5 size-4 accent-primary"
            checked={confirmed}
            onChange={(event) => setConfirmed(event.currentTarget.checked)}
          />
          <span>{checkboxLabel}</span>
        </label>
      ) : null}
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          onClick={onConfirm}
          disabled={isPending || disabled || blocked}
        >
          {isPending ? pendingLabel : confirmLabel}
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onBack}>
          {backLabel}
        </Button>
      </div>
    </div>
  );
}
