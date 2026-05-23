import * as React from "react";
import { AlertTriangle, Pencil, Plus, ShieldCheck, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { InfrastructureOwnership } from "@/lib/backendTrust";
import { PlannedBadge } from "./SettingsControls";
import {
  backendExplorerBaseUrl,
  backendProtocolLabel,
  backendTrust,
  backendsForLayer,
  explorerHostLabel,
  type Backend,
  type NetworkLayer,
} from "./SettingsModel";

export const NETWORK_LAYER_META: Record<
  NetworkLayer,
  { blurb: string; empty: string; addLabel: string }
> = {
  bitcoin: {
    blurb:
      "Explorer API, Electrum/Fulcrum, or Bitcoin Core RPC endpoints that serve on-chain history to your watch-only wallets.",
    empty:
      "No Bitcoin indexers yet. Add one so on-chain wallets can refresh their balances.",
    addLabel: "Add Bitcoin backend",
  },
  lightning: {
    blurb:
      "Read-only connections to your LND or Core Lightning node for channel accounting and profitability reports.",
    empty:
      "No Lightning nodes connected. Add a read-only LND or Core Lightning connection.",
    addLabel: "Add Lightning node",
  },
  liquid: {
    blurb:
      "Explorer API or Electrum/Fulcrum endpoints that serve Liquid (L-BTC) history to your watch-only wallets.",
    empty:
      "No Liquid indexers yet. Add one so L-BTC wallets can refresh their balances.",
    addLabel: "Add Liquid backend",
  },
};

export function NetworkLayerSettingsPanel({
  layer,
  backends,
  onAdd,
  onEdit,
  onDelete,
  onSetOwnership,
}: {
  layer: NetworkLayer;
  backends: Backend[];
  onAdd: () => void;
  onEdit: (backend: Backend) => void;
  onDelete: (backend: Backend) => void;
  onSetOwnership: (
    backend: Backend,
    ownership: InfrastructureOwnership,
  ) => Promise<void>;
}) {
  const meta = NETWORK_LAYER_META[layer];
  const layerBackends = backendsForLayer(backends, layer);
  // Infrastructure ownership is only modelled for chain indexers, matching the
  // backend dialog (Lightning connections are always your own node).
  const canOwn = layer === "bitcoin" || layer === "liquid";
  const [pendingOwnershipId, setPendingOwnershipId] = React.useState<
    string | null
  >(null);
  const handleOwnership = async (
    backend: Backend,
    ownership: InfrastructureOwnership,
  ) => {
    setPendingOwnershipId(backend.id);
    try {
      await onSetOwnership(backend, ownership);
    } finally {
      setPendingOwnershipId(null);
    }
  };
  const explorerLinkBase =
    layer === "lightning"
      ? null
      : layerBackends.map(backendExplorerBaseUrl).find(Boolean) ?? null;
  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <p className="max-w-2xl text-sm text-muted-foreground">{meta.blurb}</p>
        <Button type="button" size="sm" className="shrink-0" onClick={onAdd}>
          <Plus className="size-4" aria-hidden="true" />
          {meta.addLabel}
        </Button>
      </div>

      {layer === "lightning" ? (
        <div className="flex items-start gap-2 rounded-md border border-sky-500/25 bg-sky-500/5 p-3 text-xs text-muted-foreground">
          <ShieldCheck
            className="mt-0.5 size-4 shrink-0 text-sky-600 dark:text-sky-400"
            aria-hidden="true"
          />
          <span>
            Lightning connections are strictly read-only. Node identity details —
            operator pubkey, channel points, peer aliases, and short channel ids
            — stay on this machine.
          </span>
        </div>
      ) : null}

      {layerBackends.length === 0 ? (
        <div className="rounded-md border border-dashed bg-muted/20 p-6 text-center text-sm text-muted-foreground">
          {meta.empty}
        </div>
      ) : (
        <div className="grid gap-3">
          {layerBackends.map((backend) => (
            <BackendLayerCard
              key={backend.id}
              backend={backend}
              canOwn={canOwn}
              ownershipPending={pendingOwnershipId === backend.id}
              onEdit={() => onEdit(backend)}
              onDelete={() => onDelete(backend)}
              onSetOwnership={(ownership) =>
                void handleOwnership(backend, ownership)
              }
            />
          ))}
        </div>
      )}

      {layer === "bitcoin" || layer === "liquid" ? (
        <p className="text-xs text-muted-foreground">
          {explorerLinkBase
            ? `Transaction links open on ${explorerHostLabel(
                explorerLinkBase,
              )}; this is derived from the Explorer API backend.`
            : `Transaction links use the public ${
                layer === "bitcoin"
                  ? "mempool.bitcoin-austria.at"
                  : "Liquid Network"
              } default until you add an Explorer API backend. Electrum/Fulcrum backends are sync-only.`}
        </p>
      ) : null}
    </section>
  );
}

export function BackendLayerCard({
  backend,
  canOwn,
  ownershipPending,
  onEdit,
  onDelete,
  onSetOwnership,
}: {
  backend: Backend;
  canOwn: boolean;
  ownershipPending: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onSetOwnership: (ownership: InfrastructureOwnership) => void;
}) {
  const trust = backendTrust(backend);
  const TrustIcon = trust.icon;
  const explorerBaseUrl = backendExplorerBaseUrl(backend);
  const isSelf = backend.infrastructureOwner === "self";
  return (
    <div className="rounded-md border bg-background p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{backend.name}</span>
            {backend.isDefault ? (
              <span className="inline-flex items-center rounded-md border border-primary/25 bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary">
                Default
              </span>
            ) : null}
            {!backend.on ? (
              <span className="inline-flex items-center gap-1 rounded-md border border-amber-500/25 bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-amber-700 dark:text-amber-300">
                <AlertTriangle className="size-3" aria-hidden="true" />
                No endpoint
              </span>
            ) : null}
          </div>
          <p className="truncate font-mono text-xs text-muted-foreground">
            {backend.url}
          </p>
        </div>
        <div className="flex shrink-0 gap-1">
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={`Edit ${backend.name}`}
            onClick={onEdit}
          >
            <Pencil className="size-3.5" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={`Delete ${backend.name}`}
            onClick={onDelete}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
          </Button>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <span className="inline-flex items-center rounded-md border bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
          {backendProtocolLabel(backend)}
        </span>
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium",
            trust.className,
          )}
        >
          <TrustIcon className="size-3" aria-hidden="true" />
          {trust.label}
        </span>
        {explorerBaseUrl ? (
          <span className="inline-flex items-center rounded-md border border-sky-500/25 bg-sky-500/10 px-2 py-0.5 text-xs font-medium text-sky-700 dark:text-sky-300">
            Links: {explorerHostLabel(explorerBaseUrl)}
          </span>
        ) : null}
        {canOwn || !backend.isDefault ? (
          <div className="ml-auto flex items-center gap-1.5">
            {!backend.isDefault ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled
                title="Setting the default backend from the desktop app is coming soon — use the CLI: kassiber backends set-default"
              >
                Set as default
                <PlannedBadge className="ml-1" />
              </Button>
            ) : null}
            {canOwn ? (
              <Button
                type="button"
                size="sm"
                variant={isSelf ? "secondary" : "ghost"}
                disabled={ownershipPending}
                onClick={() => onSetOwnership(isSelf ? "third_party" : "self")}
              >
                {isSelf ? "Yours" : "Mark as mine"}
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
      <p className="mt-2 text-xs text-muted-foreground">{trust.note}</p>
    </div>
  );
}
