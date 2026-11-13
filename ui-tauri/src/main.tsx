import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";

import { AppErrorBoundary } from "@/components/AppErrorBoundary";
import {
  AppScaleController,
  ThemeController,
} from "@/components/kb/ThemeController";
import { WindowFrame } from "@/components/kb/WindowFrame";
import i18n from "@/i18n";
import { installLanguageBridge } from "@/i18n/languageBridge";
import { installGlobalErrorCapture } from "@/lib/globalErrorCapture";
import { clearBookChange } from "@/daemon/bookRefresh";
import { useUiStore } from "@/store/ui";
import { router } from "./routeTree";
import "./styles/globals.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5 * 60 * 1000,
      // A screen opened earlier in this session reopens on what it showed
      // and refetches when stale, instead of starting over on a skeleton.
      // Lock, book switch, and profile switch empty the cache (below and in
      // AppShell), so no book data outlives those boundaries.
      gcTime: 30 * 60 * 1000,
      // Neither focus nor reconnect may start a request the user did not
      // ask for. react-query defaults `refetchOnReconnect` to true, so every
      // mounted query that reaches a backend would refire on each network
      // reconnect -- the same class of unrequested traffic as a timer.
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    },
  },
});

// A new daemon session is another book or profile: drop every read of the
// previous one, and the title bar's change summary with it.
useUiStore.subscribe((state, previous) => {
  if (state.daemonSession !== previous.daemonSession) {
    queryClient.removeQueries({
      queryKey: ["daemon"],
      predicate: (query) => query.queryKey[2] !== state.daemonSession,
    });
    clearBookChange();
  } else if (state.dataMode !== previous.dataMode) {
    clearBookChange();
  }
});

installGlobalErrorCapture();
// Apply the persisted language and keep i18next in sync with the UI store.
installLanguageBridge();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <ThemeController />
        <AppScaleController />
        <WindowFrame>
          <AppErrorBoundary>
            <RouterProvider router={router} />
          </AppErrorBoundary>
        </WindowFrame>
      </QueryClientProvider>
    </I18nextProvider>
  </StrictMode>,
);
