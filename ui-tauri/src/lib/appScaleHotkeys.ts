export type AppScaleHotkeyAction = "decrease" | "increase" | "reset";

type AppScaleHotkeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey"
> & {
  shiftKey?: boolean;
};

export function appScaleHotkeyAction(
  event: AppScaleHotkeyEvent,
): AppScaleHotkeyAction | null {
  if (!(event.metaKey || event.ctrlKey) || event.altKey) return null;

  switch (event.key) {
    case "-":
    case "_":
      return "decrease";
    case "+":
    case "=":
      return "increase";
    case "0":
      return "reset";
    default:
      return null;
  }
}
