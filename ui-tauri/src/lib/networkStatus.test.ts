import { describe, expect, it } from "vitest";

import {
  networkStatusLabel,
  readNetworkStatus,
  subscribeNetworkStatus,
  type NetworkStatusEventTarget,
} from "./networkStatus";

class FakeNetworkStatusTarget implements NetworkStatusEventTarget {
  readonly listeners = {
    online: new Set<() => void>(),
    offline: new Set<() => void>(),
  };

  addEventListener(type: "online" | "offline", listener: () => void) {
    this.listeners[type].add(listener);
  }

  removeEventListener(type: "online" | "offline", listener: () => void) {
    this.listeners[type].delete(listener);
  }

  emit(type: "online" | "offline") {
    for (const listener of this.listeners[type]) {
      listener();
    }
  }
}

describe("network status", () => {
  it("reads browser online state with an online fallback outside browsers", () => {
    expect(readNetworkStatus({ onLine: true })).toBe("online");
    expect(readNetworkStatus({ onLine: false })).toBe("offline");
    expect(readNetworkStatus(undefined)).toBe("online");
  });

  it("maps status to accessible labels", () => {
    expect(networkStatusLabel("online")).toBe("Online");
    expect(networkStatusLabel("offline")).toBe("Offline");
  });

  it("tracks browser online and offline events until unsubscribed", () => {
    const target = new FakeNetworkStatusTarget();
    const nav = { onLine: true };
    const seen: string[] = [];

    const unsubscribe = subscribeNetworkStatus(
      (status) => seen.push(status),
      target,
      nav,
    );

    expect(seen).toEqual(["online"]);

    nav.onLine = false;
    target.emit("offline");
    nav.onLine = true;
    target.emit("online");

    expect(seen).toEqual(["online", "offline", "online"]);
    expect(target.listeners.online.size).toBe(1);
    expect(target.listeners.offline.size).toBe(1);

    unsubscribe();
    nav.onLine = false;
    target.emit("offline");

    expect(seen).toEqual(["online", "offline", "online"]);
    expect(target.listeners.online.size).toBe(0);
    expect(target.listeners.offline.size).toBe(0);
  });
});
