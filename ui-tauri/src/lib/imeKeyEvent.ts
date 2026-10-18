/**
 * Whether a key event belongs to an input-method composition (CJK input, dead
 * keys). Such keys confirm or edit the composition and must never trigger an
 * action such as send, select, or a shortcut.
 *
 * `isComposing` alone is not enough: some engines (Safari/WebKit among them)
 * deliver the composition-confirming Enter with `isComposing` already false
 * but the legacy `keyCode` 229 ("IME is processing").
 */
export function isImeKeyEvent(
  event:
    | Pick<KeyboardEvent, "isComposing" | "keyCode">
    | { nativeEvent: Pick<KeyboardEvent, "isComposing">; keyCode: number },
): boolean {
  const isComposing =
    "nativeEvent" in event ? event.nativeEvent.isComposing : event.isComposing;
  return isComposing === true || event.keyCode === 229;
}
