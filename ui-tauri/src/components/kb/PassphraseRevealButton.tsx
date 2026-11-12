import { Eye, EyeOff } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

/**
 * Shows or hides a passphrase field's text. The owner keeps the `revealed`
 * state in component state, so it resets whenever that screen unmounts.
 *
 * A toggle button: the accessible name stays "Show passphrase" and
 * `aria-pressed` carries the state, so a screen reader never hears
 * "Hide passphrase, pressed". The tooltip names the next action.
 */
export function PassphraseRevealButton({
  revealed,
  onToggle,
  controls,
  disabled,
}: {
  revealed: boolean;
  onToggle: () => void;
  /** The id of the input this button reveals. */
  controls: string;
  disabled?: boolean;
}) {
  const { t } = useTranslation("common");
  const showLabel = t("passphraseReveal.show");
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      aria-pressed={revealed}
      aria-controls={controls}
      aria-label={showLabel}
      title={revealed ? t("passphraseReveal.hide") : showLabel}
      disabled={disabled}
      onClick={onToggle}
    >
      {revealed ? <EyeOff aria-hidden="true" /> : <Eye aria-hidden="true" />}
    </Button>
  );
}
