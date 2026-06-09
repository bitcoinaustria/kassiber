export type NetworkStatus = "online" | "offline";

export interface NetworkStatusNavigator {
  readonly onLine: boolean;
}

export interface NetworkStatusEventTarget {
  addEventListener(type: "online" | "offline", listener: () => void): void;
  removeEventListener(type: "online" | "offline", listener: () => void): void;
}

function defaultNavigator(): NetworkStatusNavigator | undefined {
  return typeof navigator === "undefined" ? undefined : navigator;
}

function defaultEventTarget(): NetworkStatusEventTarget | undefined {
  return typeof window === "undefined" ? undefined : window;
}

export function readNetworkStatus(
  nav: NetworkStatusNavigator | undefined = defaultNavigator(),
): NetworkStatus {
  return nav?.onLine === false ? "offline" : "online";
}

export function networkStatusLabel(status: NetworkStatus) {
  return status === "online" ? "Online" : "Offline";
}

export function subscribeNetworkStatus(
  onChange: (status: NetworkStatus) => void,
  target: NetworkStatusEventTarget | undefined = defaultEventTarget(),
  nav: NetworkStatusNavigator | undefined = defaultNavigator(),
) {
  const syncOnlineStatus = () => onChange(readNetworkStatus(nav));

  syncOnlineStatus();
  if (!target) return () => {};

  target.addEventListener("online", syncOnlineStatus);
  target.addEventListener("offline", syncOnlineStatus);
  return () => {
    target.removeEventListener("online", syncOnlineStatus);
    target.removeEventListener("offline", syncOnlineStatus);
  };
}
