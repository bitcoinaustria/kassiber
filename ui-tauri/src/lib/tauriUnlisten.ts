type MaybePromise = void | Promise<void>;

export type TauriUnlisten = () => MaybePromise;

export function safeTauriUnlisten(
  unlisten: TauriUnlisten | null | undefined,
): void {
  if (!unlisten) return;
  try {
    const result = unlisten();
    if (result && typeof result.catch === "function") {
      void result.catch((error: unknown) => {
        console.warn("Could not unregister Tauri event listener", error);
      });
    }
  } catch (error) {
    console.warn("Could not unregister Tauri event listener", error);
  }
}
