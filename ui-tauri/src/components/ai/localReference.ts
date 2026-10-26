/** Open a tool result's in-app reference, such as one transaction. */
export function openLocalReference(reference: {
  route: "/transactions";
  transaction: string;
}) {
  if (typeof window === "undefined") return;
  const target = `${reference.route}?tx=${encodeURIComponent(reference.transaction)}`;
  window.history.pushState({}, "", target);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

/** App pages an assistant visual may link to; anything else is not opened. */
const APP_ROUTES = new Set(["/overview", "/reports", "/transactions", "/quarantine"]);

export function openAppRoute(route: string) {
  if (typeof window === "undefined" || !APP_ROUTES.has(route)) return;
  window.history.pushState({}, "", route);
  window.dispatchEvent(new PopStateEvent("popstate"));
}
