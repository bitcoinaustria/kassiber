export async function confirmAction(message: string): Promise<boolean> {
  if (typeof window === "undefined") return false;
  return Promise.resolve(window.confirm(message));
}
