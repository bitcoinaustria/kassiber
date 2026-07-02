/**
 * Connections list view.
 *
 * Uses the shared shadcn dashboard language while keeping row navigation.
 */

import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import {
  WalletsDashboardHeader,
  WalletsFilters,
  WalletsMetricGrid,
  WalletsTable,
} from "@/components/kb/wallets";
import { regtestBackendConnections } from "@/components/kb/backendConnectionRows";
import { PENDING_SETTINGS_BACKEND_EDIT_KEY } from "@/components/kb/settingsSections";
import { useDaemon } from "@/daemon/client";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { connectionCategoryLabel } from "@/lib/connectionDisplay";
import { useCurrency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { useUiStore } from "@/store/ui";
import {
  backendRowToSettingsBackend,
  type BackendSettingsData,
} from "@/components/kb/settings/SettingsModel";

import type {
  Connection,
  ConnectionStatus,
  OverviewSnapshot,
} from "@/mocks/seed";

export function Connections() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
  );
  const { isSyncing } = useWalletSyncAction();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [addConnectionOpen, setAddConnectionOpen] = useState(false);
  const [resumeSourceId, setResumeSourceId] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<string | "all">("all");
  const [statusFilter, setStatusFilter] = useState<ConnectionStatus | "all">(
    "all",
  );
  const deferredConnectionSetup = useUiStore(
    (s) => s.deferredConnectionSetup,
  );
  const clearDeferredConnectionSetup = useUiStore(
    (s) => s.clearDeferredConnectionSetup,
  );

  useEffect(() => {
    if (!deferredConnectionSetup) return;
    setResumeSourceId(deferredConnectionSetup.sourceId);
    setAddConnectionOpen(true);
    clearDeferredConnectionSetup();
  }, [deferredConnectionSetup, clearDeferredConnectionSetup]);

  if (isLoading || !data?.data) {
    return <ScreenSkeleton titleWidth="w-32" metricCount={3} />;
  }

  const snapshot = data.data;
  const backendRows =
    backendSettingsQuery.data?.data?.backends.map(backendRowToSettingsBackend) ??
    [];
  const backendConnections = regtestBackendConnections(backendRows);
  const connections: Connection[] = [
    ...snapshot.connections,
    ...backendConnections,
  ];
  const totalBtc = snapshot.connections.reduce((s, c) => s + c.balance, 0);
  const filteredConnections = connections.filter(
    (connection) =>
      (kindFilter === "all" ||
        connectionCategoryLabel(connection) === kindFilter) &&
      (statusFilter === "all" || connection.status === statusFilter),
  );
  const hasActiveFilters = kindFilter !== "all" || statusFilter !== "all";
  const clearFilters = () => {
    setKindFilter("all");
    setStatusFilter("all");
  };
  const onSelectConnection = (id: string) => {
    const connection = connections.find((row) => row.id === id);
    if (connection?.role === "backend" && connection.backendId) {
      window.sessionStorage.setItem(
        PENDING_SETTINGS_BACKEND_EDIT_KEY,
        connection.backendId,
      );
      void navigate({
        to: "/settings",
        hash: connection.settingsHash ?? "bitcoin",
      });
      return;
    }
    void navigate({
      to: "/connections/$connectionId",
      params: { connectionId: id },
    });
  };

  return (
    <div className={screenShellClassName}>
      <WalletsDashboardHeader
        onAddWallet={() => setAddConnectionOpen(true)}
      />
      <AddConnectionDialog
        open={addConnectionOpen}
        onOpenChange={(next) => {
          setAddConnectionOpen(next);
          if (!next) setResumeSourceId(null);
        }}
        initialSourceId={resumeSourceId}
      />
      <WalletsMetricGrid
        connections={snapshot.connections}
        currency={currency}
        hideSensitive={hideSensitive}
        isSyncing={isSyncing}
        priceEur={snapshot.priceEur}
        totalBtc={totalBtc}
      />

      <div className="rounded-xl border bg-card">
        <WalletsFilters
          filteredCount={filteredConnections.length}
          hasActiveFilters={hasActiveFilters}
          kindFilter={kindFilter}
          onClearFilters={clearFilters}
          onKindFilterChange={setKindFilter}
          onStatusFilterChange={setStatusFilter}
          statusFilter={statusFilter}
        />
        <WalletsTable
          connections={filteredConnections}
          currency={currency}
          hideSensitive={hideSensitive}
          onSelectConnection={onSelectConnection}
          priceEur={snapshot.priceEur}
          totalBtc={totalBtc}
          totalCount={connections.length}
        />
      </div>
    </div>
  );
}
