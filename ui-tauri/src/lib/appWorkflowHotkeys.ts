export type AppWorkflowHotkeyAction =
  | "add-wallet"
  | "refresh-book"
  | "rescan-book"
  | "process-journals";

type AppWorkflowHotkeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
> & {
  repeat?: boolean;
};

export function appWorkflowHotkeyAction(
  event: AppWorkflowHotkeyEvent,
): AppWorkflowHotkeyAction | null {
  if (!(event.metaKey || event.ctrlKey) || event.altKey || event.repeat) {
    return null;
  }

  const key = event.key.toLowerCase();
  if (event.shiftKey && key === "a") return "add-wallet";
  if (key === "r") {
    return event.shiftKey ? "rescan-book" : "refresh-book";
  }
  if (event.shiftKey && key === "s") return "refresh-book";
  if (event.shiftKey && key === "j") return "process-journals";

  return null;
}
