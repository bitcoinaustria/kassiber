/**
 * Authenticated layout: AppHeader + Outlet + AppFooter.
 *
 * Routes that mount under the authenticated layout (Overview,
 * Transactions, Reports, Profiles, Connections) get full app chrome.
 * Welcome and any future onboarding screens live outside this shell.
 */

import { Outlet } from "@tanstack/react-router";
import { AppHeader } from "./AppHeader";
import { AppFooter } from "./AppFooter";

export function AppShell() {
  return (
    <div className="flex min-h-screen flex-col bg-paper">
      <AppHeader />
      <main className="flex min-h-0 flex-1 flex-col">
        <Outlet />
      </main>
      <AppFooter />
    </div>
  );
}
