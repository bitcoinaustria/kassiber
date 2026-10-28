// @vitest-environment happy-dom
//
// Settings opens as a modal over the page that asked for it: a navigation to
// any `/settings/*` route opens (or switches) the modal instead of leaving the
// page, adds no history entry, and keeps the developer-tools guard.
import {
  Link,
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
} from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const makeStorage = () => {
    const storage = new Map<string, string>();
    return {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => void storage.set(key, value),
      removeItem: (key: string) => void storage.delete(key),
    };
  };
  vi.stubGlobal("localStorage", makeStorage());
  vi.stubGlobal("sessionStorage", makeStorage());
});

vi.mock("@/components/kb/SettingsScreen", () => ({
  SettingsScreen: ({ sectionId }: { sectionId: string }) => (
    <div data-testid="settings-screen">{sectionId}</div>
  ),
}));
vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({ data: undefined }),
}));

import { useUiStore } from "@/store/ui";

import { SettingsModal } from "./SettingsModal";
import {
  openSettingsSection,
  settingsSectionForLocation,
  useSettingsModalNavigation,
} from "./settingsModal";

function Shell({ unlocked }: { unlocked: boolean }) {
  useSettingsModalNavigation(unlocked);
  const navigate = useNavigate();
  return (
    <>
      <Link to="/settings/market">Market settings</Link>
      <button
        type="button"
        onClick={() => void navigate({ to: "/settings", hash: "backends" })}
      >
        Legacy hash
      </button>
      <button type="button" onClick={() => void navigate({ to: "/logs" })}>
        Open logs
      </button>
      <Outlet />
      <SettingsModal daemonEnabled />
    </>
  );
}

function mount({ unlocked = true }: { unlocked?: boolean } = {}) {
  const rootRoute = createRootRoute({ component: () => <Shell unlocked={unlocked} /> });
  const page = (path: string) =>
    createRoute({
      getParentRoute: () => rootRoute,
      path,
      component: () => <p>page {path}</p>,
    });
  const history = createMemoryHistory({ initialEntries: ["/transactions"] });
  const router = createRouter({
    routeTree: rootRoute.addChildren([
      page("/transactions"),
      page("/logs"),
      page("/settings"),
      page("/settings/market"),
    ]),
    history,
  });
  render(<RouterProvider router={router} />);
  return { router, history };
}

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe("settingsSectionForLocation", () => {
  it.each([
    ["/settings/market", "", "network-market"],
    ["/settings/market/", "", "network-market"],
    ["/settings", "#backends", "network-bitcoin"],
    ["/settings", "", "general-appearance"],
    ["/settings/nope", "", "general-appearance"],
    ["/transactions", "", null],
    ["/settingsx", "", null],
  ])("maps %s%s to %s", (pathname, hash, section) => {
    expect(settingsSectionForLocation(pathname, hash)).toBe(section);
  });
});

describe("SettingsModal", () => {
  beforeEach(() => {
    useUiStore.setState({ settingsSection: null, developerToolsEnabled: false });
  });
  afterEach(cleanup);

  it("opens over the page from a settings link, without leaving it", async () => {
    const { router, history } = mount();
    await settle();
    expect(screen.getByText("page /transactions")).toBeTruthy();
    const entries = history.length;

    fireEvent.click(screen.getByRole("link", { name: "Market settings" }));
    await settle();

    const modal = screen.getByRole("dialog", { name: "Settings" });
    expect(modal.hasAttribute("data-app-modal")).toBe(true);
    expect(screen.getByTestId("settings-screen").textContent).toBe("network-market");
    // The page stays mounted underneath, and the history did not move.
    expect(screen.getByText("page /transactions")).toBeTruthy();
    expect(router.state.location.pathname).toBe("/transactions");
    expect(history.length).toBe(entries);
  });

  it("drops a request for Settings made behind the lock screen", async () => {
    const { router } = mount({ unlocked: false });
    await settle();
    fireEvent.click(screen.getByRole("link", { name: "Market settings" }));
    await settle();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useUiStore.getState().settingsSection).toBeNull();
    expect(router.state.location.pathname).toBe("/transactions");
  });

  it("switches categories in place from its rail", async () => {
    mount();
    await settle();
    fireEvent.click(screen.getByRole("button", { name: "Legacy hash" }));
    await settle();
    expect(screen.getByTestId("settings-screen").textContent).toBe("network-bitcoin");

    fireEvent.click(screen.getByRole("button", { name: "Privacy" }));
    expect(screen.getByTestId("settings-screen").textContent).toBe("security-privacy");
    expect(
      screen.getByRole("button", { name: "Privacy" }).getAttribute("aria-current"),
    ).toBe("page");
  });

  it("closes from its close button and from Escape", async () => {
    mount();
    await settle();
    act(() => void openSettingsSection("network-market"));
    fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useUiStore.getState().settingsSection).toBeNull();

    act(() => void openSettingsSection("network-market"));
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes when the user leaves the page from inside it", async () => {
    const { router } = mount();
    await settle();
    act(() => void openSettingsSection("desktop-developer"));
    fireEvent.click(screen.getByRole("button", { name: "Open logs", hidden: true }));
    await settle();
    expect(router.state.location.pathname).toBe("/logs");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("keeps developer-only categories shut while developer tools are off", async () => {
    mount();
    await settle();
    expect(openSettingsSection("network-lightning")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("button", { name: "Lightning" })).toBeNull();

    act(() => useUiStore.setState({ developerToolsEnabled: true }));
    act(() => void openSettingsSection("network-lightning"));
    expect(screen.getByTestId("settings-screen").textContent).toBe("network-lightning");

    // Turning the switch off while there falls back to the default category.
    act(() => useUiStore.setState({ developerToolsEnabled: false }));
    expect(screen.getByTestId("settings-screen").textContent).toBe("general-appearance");
  });
});
