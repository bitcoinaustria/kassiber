import { useUiStore } from "@/store/ui";

const CLIPBOARD_CLEAR_DELAY_MS = 30_000;

export async function copyTextWithPolicy(value: string): Promise<void> {
  if (typeof navigator === "undefined" || !navigator.clipboard) return;
  await navigator.clipboard.writeText(value);
  if (!useUiStore.getState().clearClipboard) return;
  const schedule =
    typeof window !== "undefined"
      ? window.setTimeout.bind(window)
      : globalThis.setTimeout;

  schedule(() => {
    void navigator.clipboard
      ?.readText()
      .then((current) => {
        if (current === value) {
          return navigator.clipboard?.writeText("");
        }
        return undefined;
      })
      .catch(() => undefined);
  }, CLIPBOARD_CLEAR_DELAY_MS);
}
