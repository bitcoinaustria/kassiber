/**
 * SettingsModal — workspace-wide preferences.
 *
 * Translated from claude-design/screens/settings.jsx. Sections:
 *  - Privacy (hide-sensitive toggle wired to the Zustand UI store; the
 *    clipboard auto-clear toggle is local UI state for now)
 *  - App lock (auto-lock, idle timeout, passphrase prompts) — local
 *    state until the lock subsystem lands
 *  - Data (backup/restore/logs stubs, BIP-329 + CSV import/export
 *    stubs, DB path readout)
 *  - Sync backends (list + Add backend sub-modal)
 *  - Danger zone (workspace reset stub)
 *
 * Add-backend flow is a nested modal, same `Dialog` shell with the
 * kassiber hard-edge override.
 */
import * as React from "react";
import { Lock } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { LabeledInput } from "@/components/kb/LabeledInput";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

type Net = "BTC" | "LIQUID" | "LN" | "FX";

interface Backend {
  id: string;
  name: string;
  url: string;
  net: Net;
  health: string;
  on: boolean;
  auth: string;
}

const DEFAULT_BACKENDS: Backend[] = [
  {
    id: "b1",
    name: "mempool.space",
    url: "https://mempool.space/api",
    net: "BTC",
    health: "#893,014 · 2m",
    on: true,
    auth: "none",
  },
  {
    id: "b2",
    name: "local electrs",
    url: "tcp://127.0.0.1:50001",
    net: "BTC",
    health: "—",
    on: false,
    auth: "none",
  },
  {
    id: "b3",
    name: "Blockstream Liquid",
    url: "https://blockstream.info/liquid/api",
    net: "LIQUID",
    health: "—",
    on: false,
    auth: "none",
  },
  {
    id: "b4",
    name: "CoinGecko",
    url: "https://api.coingecko.com/api/v3",
    net: "FX",
    health: "€71,420 · 14s",
    on: true,
    auth: "none",
  },
];

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);

  const [clearClipboard, setClearClipboard] = React.useState(true);

  const [autoLockEnabled, setAutoLockEnabled] = React.useState(true);
  const [autoLockMinutes, setAutoLockMinutes] = React.useState(5);
  const [requirePassphrase, setRequirePassphrase] = React.useState(true);
  const [lockOnClose, setLockOnClose] = React.useState(true);

  const [backends, setBackends] = React.useState<Backend[]>(DEFAULT_BACKENDS);
  const [addOpen, setAddOpen] = React.useState(false);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        className={cn(
          "max-h-[88vh] w-full max-w-[580px] gap-0 overflow-y-auto",
          "rounded-none border-ink bg-paper p-0 shadow-hard-ink",
          "data-[state=open]:zoom-in-100 data-[state=closed]:zoom-out-100",
        )}
      >
        <DialogHeader className="flex-row items-center justify-between gap-2 border-b border-line px-5 py-3.5">
          <DialogTitle className="font-sans text-base font-semibold tracking-[-0.005em] text-ink">
            Settings
          </DialogTitle>
          <DialogDescription className="sr-only">
            Workspace preferences for Kassiber.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-5 p-5">
          {/* Privacy */}
          <Section title="Privacy">
            <Row
              label="Hide sensitive data"
              sub="Blur balances, addresses, and amounts throughout the UI."
              control={
                <Toggle on={hideSensitive} onChange={setHideSensitive} />
              }
            />
            <Row
              label="Clear clipboard after 30s"
              sub="Auto-clear copied addresses and keys."
              control={
                <Toggle on={clearClipboard} onChange={setClearClipboard} />
              }
            />
          </Section>

          {/* App lock */}
          <Section title="App lock">
            <Row
              label="Auto-lock when idle"
              sub="Require passphrase to re-enter after a period of inactivity."
              control={
                <Toggle on={autoLockEnabled} onChange={setAutoLockEnabled} />
              }
            />
            <div
              className={cn(
                "flex items-center gap-2 px-0.5 py-1",
                !autoLockEnabled && "opacity-40",
              )}
            >
              <span className="font-sans text-xs text-ink-2">Idle timeout</span>
              <div className="ml-auto flex gap-1">
                {[1, 5, 15, 30, 60].map((m) => (
                  <Pill
                    key={m}
                    active={autoLockMinutes === m}
                    onClick={
                      autoLockEnabled
                        ? () => setAutoLockMinutes(m)
                        : undefined
                    }
                  >
                    {m}m
                  </Pill>
                ))}
              </div>
            </div>
            <Row
              label="Require passphrase on launch"
              sub="Prompt for your workspace passphrase every time Kassiber opens."
              control={
                <Toggle
                  on={requirePassphrase}
                  onChange={setRequirePassphrase}
                />
              }
            />
            <Row
              label="Lock on window close"
              sub="Clear in-memory decrypted state when the app window is closed."
              control={<Toggle on={lockOnClose} onChange={setLockOnClose} />}
            />
            <div className="mt-1 flex gap-2">
              <KbSecondaryButton size="sm">
                <Lock className="size-2.5" />
                Lock now
              </KbSecondaryButton>
              <KbGhostButton size="sm">Change passphrase…</KbGhostButton>
            </div>
          </Section>

          {/* Data */}
          <Section title="Data">
            <div className="grid grid-cols-3 gap-2">
              <KbSecondaryButton size="md" align="start">
                <span className="font-mono text-sm">⤓</span>
                Backup
              </KbSecondaryButton>
              <KbSecondaryButton size="md" align="start">
                <span className="font-mono text-sm">⤒</span>
                Restore
              </KbSecondaryButton>
              <KbSecondaryButton size="md" align="start">
                <span className="font-mono text-sm">⋯</span>
                Logs
              </KbSecondaryButton>
            </div>

            <div className="mt-2.5 border border-line bg-paper px-3 py-2.5">
              <div className="mb-2 font-mono text-[9px] font-semibold uppercase tracking-[0.14em] text-ink-3">
                Labels & imports · workspace-wide
              </div>
              <div className="grid grid-cols-3 gap-2">
                <KbSecondaryButton size="sm" align="start">
                  ↓ Import BIP-329
                </KbSecondaryButton>
                <KbSecondaryButton size="sm" align="start">
                  ↑ Export BIP-329
                </KbSecondaryButton>
                <KbSecondaryButton size="sm" align="start">
                  ↓ Import CSV
                </KbSecondaryButton>
              </div>
            </div>

            <div className="mt-2 font-mono text-[10px] leading-[1.6] text-ink-3">
              DB ~/.kassiber/kassiber.db · 2.4 MB
              <br />
              Last backup 2026-04-17 23:02 · backup_2026-04-17.tar.zst
            </div>
          </Section>

          {/* Sync backends */}
          <Section title="Sync backends">
            <div className="flex flex-col gap-1.5">
              {backends.map((b) => (
                <div
                  key={b.id}
                  className="grid grid-cols-[10px_64px_1fr_auto_auto] items-center gap-3 border border-line px-2.5 py-2"
                >
                  <span
                    className={cn(
                      "size-1.5 rounded-full",
                      b.on ? "bg-[#3fa66a]" : "bg-ink-3",
                    )}
                  />
                  <NetworkBadge net={b.net} />
                  <div className="min-w-0">
                    <div className="font-mono text-xs text-ink">{b.name}</div>
                    <div className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[10px] text-ink-3">
                      {b.url}
                    </div>
                  </div>
                  <span
                    className={cn(
                      "min-w-[120px] text-right font-mono text-[10px] tracking-[0.04em]",
                      b.on ? "text-ink-2" : "text-ink-3",
                    )}
                  >
                    {b.health}
                  </span>
                  <span
                    className={cn(
                      "min-w-[44px] text-right font-mono text-[9px] uppercase tracking-[0.12em]",
                      b.on ? "text-[#3fa66a]" : "text-ink-3",
                    )}
                  >
                    {b.on ? "active" : "idle"}
                  </span>
                </div>
              ))}

              <button
                onClick={() => setAddOpen(true)}
                className="flex cursor-pointer items-center justify-center gap-2 border border-dashed border-ink-3 bg-transparent px-2.5 py-2.5 font-mono text-[11px] uppercase tracking-[0.1em] text-ink-2 hover:border-ink hover:bg-paper-2"
              >
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 10 10"
                  fill="none"
                  aria-hidden="true"
                >
                  <path
                    d="M5 1 V9 M1 5 H9"
                    stroke="currentColor"
                    strokeWidth="1.2"
                    strokeLinecap="round"
                  />
                </svg>
                Add backend
              </button>
            </div>
          </Section>

          {/* Danger zone */}
          <Section title="Danger zone">
            <Button
              variant="destructive"
              className="self-start rounded-none"
              size="sm"
            >
              ⚠ Reset workspace
            </Button>
          </Section>
        </div>

        <AddBackendModal
          open={addOpen}
          onClose={() => setAddOpen(false)}
          onAdd={(b) => {
            setBackends((prev) => [...prev, b]);
            setAddOpen(false);
          }}
        />
      </DialogContent>
    </Dialog>
  );
}

interface SectionProps {
  title: string;
  children: React.ReactNode;
}

function Section({ title, children }: SectionProps) {
  return (
    <div className="flex flex-col gap-2.5">
      <div className="border-b border-line pb-1.5 font-mono text-[10px] font-semibold uppercase tracking-[0.14em] text-ink-3">
        {title}
      </div>
      {children}
    </div>
  );
}

interface RowProps {
  label: string;
  sub?: string;
  control: React.ReactNode;
}

function Row({ label, sub, control }: RowProps) {
  return (
    <div className="flex items-center gap-3.5 py-1.5">
      <div className="flex-1">
        <div className="font-sans text-[13px] text-ink">{label}</div>
        {sub && (
          <div className="mt-0.5 font-sans text-[11px] text-ink-3">{sub}</div>
        )}
      </div>
      {control}
    </div>
  );
}

interface ToggleProps {
  on: boolean;
  onChange?: (next: boolean) => void;
}

function Toggle({ on, onChange }: ToggleProps) {
  return (
    <button
      type="button"
      onClick={() => onChange?.(!on)}
      className={cn(
        "relative h-5 w-9 cursor-pointer border-none p-0 transition-colors",
        on ? "bg-ink" : "bg-line-2",
      )}
      aria-pressed={on}
    >
      <span
        className={cn(
          "absolute top-0.5 size-4 bg-paper-2 transition-[left]",
          on ? "left-[18px]" : "left-0.5",
        )}
      />
    </button>
  );
}

interface PillProps {
  active: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}

function Pill({ active, onClick, children }: PillProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!onClick}
      className={cn(
        "border px-2 py-0.5 font-mono text-[10px] tracking-[0.04em] disabled:cursor-not-allowed",
        active
          ? "border-ink bg-ink text-paper"
          : "border-line bg-transparent text-ink-2 hover:border-ink-3",
        onClick && "cursor-pointer",
      )}
    >
      {children}
    </button>
  );
}

interface NetworkBadgeProps {
  net: Net;
}

const NETWORK_PALETTE: Record<
  Net,
  { className: string; style?: React.CSSProperties }
> = {
  BTC: {
    className:
      "border-[rgba(177,106,18,0.45)] bg-[rgba(177,106,18,0.10)] text-[#b16a12]",
  },
  LIQUID: {
    className:
      "border-[rgba(62,94,168,0.45)] bg-[rgba(62,94,168,0.10)] text-[#3e5ea8]",
  },
  LN: {
    className:
      "border-[rgba(122,63,166,0.45)] bg-[rgba(122,63,166,0.10)] text-[#7a3fa6]",
  },
  FX: {
    className: "border-ink-3 bg-transparent text-ink-2",
  },
};

function NetworkBadge({ net }: NetworkBadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center justify-center border px-2 py-0.5 font-mono text-[9px] font-bold tracking-[0.14em]",
        NETWORK_PALETTE[net].className,
      )}
    >
      {net}
    </span>
  );
}

interface KbButtonProps {
  size?: "sm" | "md";
  align?: "center" | "start";
  disabled?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
}

function KbSecondaryButton({
  size = "md",
  align = "center",
  disabled,
  onClick,
  children,
}: KbButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "inline-flex cursor-pointer items-center gap-2 border border-line bg-paper-2 font-sans text-ink hover:border-ink hover:bg-paper",
        align === "start" ? "justify-start" : "justify-center",
        size === "sm"
          ? "h-7 px-2.5 text-[11px]"
          : "h-8 px-3 text-xs",
        disabled && "cursor-not-allowed opacity-50 hover:border-line hover:bg-paper-2",
      )}
    >
      {children}
    </button>
  );
}

function KbGhostButton({
  size = "md",
  onClick,
  children,
}: KbButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex cursor-pointer items-center justify-center gap-2 border border-transparent bg-transparent font-sans text-ink-2 hover:text-ink",
        size === "sm" ? "h-7 px-2 text-[11px]" : "h-8 px-2.5 text-xs",
      )}
    >
      {children}
    </button>
  );
}

// ─── Add Backend modal ───────────────────────────────────────────────────────

interface BackendPreset {
  id: string;
  name: string;
  url: string;
  scheme: string;
}

interface BackendType {
  id: string;
  label: string;
  net: Net;
  desc: string;
  presets: BackendPreset[];
}

const BACKEND_TYPES: BackendType[] = [
  {
    id: "btc",
    label: "Bitcoin node",
    net: "BTC",
    desc: "Read blocks, addresses and UTXOs from a Bitcoin backend.",
    presets: [
      { id: "mempool", name: "mempool.space", url: "https://mempool.space/api", scheme: "REST" },
      { id: "esplora", name: "Blockstream Esplora", url: "https://blockstream.info/api", scheme: "REST" },
      { id: "electrum", name: "Electrum server", url: "tcp://127.0.0.1:50001", scheme: "Electrum" },
      { id: "core", name: "Bitcoin Core RPC", url: "http://127.0.0.1:8332", scheme: "RPC" },
    ],
  },
  {
    id: "lightning",
    label: "Lightning",
    net: "LN",
    desc: "Read channel state, invoices and forwards from an LN node.",
    presets: [
      { id: "lnd", name: "LND", url: "https://127.0.0.1:8080", scheme: "REST" },
      { id: "cln", name: "Core Lightning", url: "http://127.0.0.1:3010", scheme: "CLNREST" },
      { id: "lnbits", name: "LNbits", url: "https://your.lnbits.host", scheme: "REST" },
      { id: "nwc", name: "Nostr Wallet Connect", url: "nostr+walletconnect://", scheme: "NWC" },
    ],
  },
  {
    id: "liquid",
    label: "Liquid / sidechain",
    net: "LIQUID",
    desc: "Read Liquid, Rootstock or other sidechain balances.",
    presets: [
      { id: "blockstream", name: "Blockstream Liquid", url: "https://blockstream.info/liquid/api", scheme: "REST" },
      { id: "liquidcore", name: "Elements RPC", url: "http://127.0.0.1:7041", scheme: "RPC" },
    ],
  },
  {
    id: "fx",
    label: "Price / FX",
    net: "FX",
    desc: "BTC/EUR and other fiat reference rates, spot and historical.",
    presets: [
      { id: "coingecko", name: "CoinGecko", url: "https://api.coingecko.com/api/v3", scheme: "REST" },
      { id: "kraken", name: "Kraken", url: "https://api.kraken.com/0/public", scheme: "REST" },
      { id: "bitstamp", name: "Bitstamp", url: "https://www.bitstamp.net/api/v2", scheme: "REST" },
      { id: "ecb", name: "ECB reference", url: "https://data-api.ecb.europa.eu/service/data", scheme: "REST" },
    ],
  },
  {
    id: "other",
    label: "Other",
    net: "FX",
    desc: "A generic HTTP / WebSocket endpoint.",
    presets: [],
  },
];

const AUTH_MODES: Array<{ id: string; label: string }> = [
  { id: "none", label: "None" },
  { id: "apikey", label: "API key" },
  { id: "basic", label: "User + pass" },
  { id: "bearer", label: "Bearer token" },
];

type TestState = "idle" | "testing" | "ok" | "fail";

interface AddBackendModalProps {
  open: boolean;
  onClose: () => void;
  onAdd: (backend: Backend) => void;
}

function AddBackendModal({ open, onClose, onAdd }: AddBackendModalProps) {
  const [typeId, setTypeId] = React.useState("btc");
  const [presetId, setPresetId] = React.useState("mempool");
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState("https://mempool.space/api");
  const [auth, setAuth] = React.useState("none");
  const [authVal, setAuthVal] = React.useState("");
  const [authVal2, setAuthVal2] = React.useState("");
  const [testState, setTestState] = React.useState<TestState>("idle");

  const type =
    BACKEND_TYPES.find((t) => t.id === typeId) ?? BACKEND_TYPES[0];
  const preset =
    presetId === "custom"
      ? null
      : type.presets.find((p) => p.id === presetId) ?? null;

  React.useEffect(() => {
    if (!open) return;
    setTypeId("btc");
    setPresetId("mempool");
    setName("");
    setUrl("https://mempool.space/api");
    setAuth("none");
    setAuthVal("");
    setAuthVal2("");
    setTestState("idle");
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    if (preset) {
      setUrl(preset.url);
      setName(preset.name);
    } else if (presetId === "custom") {
      setUrl("");
      setName("");
    }
    setTestState("idle");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [typeId, presetId]);

  const onPickType = (id: string) => {
    setTypeId(id);
    const t = BACKEND_TYPES.find((x) => x.id === id);
    setPresetId(t?.presets[0]?.id ?? "custom");
  };

  const testConnection = () => {
    if (!url.trim()) return;
    setTestState("testing");
    setTimeout(() => {
      const ok = /^(https?|tcp|wss?|nostr\+walletconnect):\/\/[\w.\-:/]+/i.test(
        url.trim(),
      );
      setTestState(ok ? "ok" : "fail");
    }, 900);
  };

  const canAdd = name.trim().length > 0 && url.trim().length > 0;
  const add = () => {
    if (!canAdd) return;
    onAdd({
      id: "b" + Date.now(),
      name: name.trim(),
      url: url.trim(),
      net: type.net,
      health: testState === "ok" ? "just added · ok" : "—",
      on: testState === "ok",
      auth,
    });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        className={cn(
          "max-h-[90vh] w-full max-w-[620px] gap-0 overflow-y-auto",
          "rounded-none border-ink bg-paper p-0 shadow-hard-ink",
        )}
      >
        <DialogHeader className="flex-row items-center justify-between gap-2 border-b border-line px-5 py-3.5">
          <DialogTitle className="font-sans text-base font-semibold tracking-[-0.005em] text-ink">
            Add backend
          </DialogTitle>
          <DialogDescription className="sr-only">
            Connect a Bitcoin, Lightning, Liquid, or rate backend.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-[18px] p-5">
          {/* 1 — Type selector */}
          <div>
            <SectionLabel step="01" label="Backend type" />
            <div className="mt-2 grid grid-cols-5 gap-1.5">
              {BACKEND_TYPES.map((t) => {
                const active = t.id === typeId;
                return (
                  <button
                    key={t.id}
                    type="button"
                    onClick={() => onPickType(t.id)}
                    className={cn(
                      "flex min-h-[72px] cursor-pointer flex-col gap-1.5 border p-2.5 text-left",
                      active
                        ? "border-ink bg-paper-2 shadow-[3px_3px_0_var(--color-ink)]"
                        : "border-line bg-transparent",
                    )}
                  >
                    <NetworkBadge net={t.net} />
                    <span className="font-sans text-xs font-medium leading-[1.2] text-ink">
                      {t.label}
                    </span>
                  </button>
                );
              })}
            </div>
            <div className="mt-2 font-sans text-[11px] text-ink-3">
              {type.desc}
            </div>
          </div>

          {/* 2 — Preset */}
          {type.presets.length > 0 && (
            <div>
              <SectionLabel step="02" label="Preset" />
              <div className="mt-2 flex flex-wrap gap-1">
                {type.presets.map((p) => (
                  <Pill
                    key={p.id}
                    active={presetId === p.id}
                    onClick={() => setPresetId(p.id)}
                  >
                    {p.name}
                    <span className="ml-1.5 text-[9px] opacity-60">
                      {p.scheme}
                    </span>
                  </Pill>
                ))}
                <Pill
                  active={presetId === "custom"}
                  onClick={() => setPresetId("custom")}
                >
                  + Custom
                </Pill>
              </div>
            </div>
          )}

          {/* 3 — Connection details */}
          <div>
            <SectionLabel
              step={type.presets.length > 0 ? "03" : "02"}
              label="Connection"
            />
            <div className="mt-2 flex flex-col gap-2.5">
              <LabeledInput
                label="Display name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. My home node"
              />
              <LabeledInput
                label="Endpoint URL"
                value={url}
                onChange={(e) => {
                  setUrl(e.target.value);
                  setTestState("idle");
                }}
                placeholder="https://…"
                mono
              />

              <div className="flex flex-col gap-1.5">
                <span className="font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-2">
                  Authentication
                </span>
                <div className="flex flex-wrap gap-1">
                  {AUTH_MODES.map((m) => (
                    <Pill
                      key={m.id}
                      active={auth === m.id}
                      onClick={() => setAuth(m.id)}
                    >
                      {m.label}
                    </Pill>
                  ))}
                </div>
                {auth === "apikey" && (
                  <LabeledInput
                    label="API key"
                    value={authVal}
                    onChange={(e) => setAuthVal(e.target.value)}
                    placeholder="sk_live_…"
                    type="password"
                    mono
                  />
                )}
                {auth === "bearer" && (
                  <LabeledInput
                    label="Bearer token"
                    value={authVal}
                    onChange={(e) => setAuthVal(e.target.value)}
                    placeholder="eyJ…"
                    type="password"
                    mono
                  />
                )}
                {auth === "basic" && (
                  <div className="grid grid-cols-2 gap-2">
                    <LabeledInput
                      label="Username"
                      value={authVal}
                      onChange={(e) => setAuthVal(e.target.value)}
                      mono
                    />
                    <LabeledInput
                      label="Password"
                      value={authVal2}
                      onChange={(e) => setAuthVal2(e.target.value)}
                      type="password"
                      mono
                    />
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Test + footer */}
          <div className="flex items-center gap-2.5 border-t border-ink pt-3.5">
            <KbSecondaryButton
              size="sm"
              onClick={testConnection}
              disabled={!url.trim() || testState === "testing"}
            >
              <span
                className={cn(
                  "inline-flex size-2.5 items-center justify-center",
                  testState === "testing" && "animate-spin",
                )}
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path
                    d="M1.6 5 A3.4 3.4 0 1 1 5 8.4"
                    stroke="currentColor"
                    strokeWidth="1.1"
                    fill="none"
                    strokeLinecap="round"
                  />
                  <path
                    d="M1.6 5 L1.6 2.3 L4.3 2.3"
                    stroke="currentColor"
                    strokeWidth="1.1"
                    fill="none"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </span>
              {testState === "testing" ? "Testing…" : "Test connection"}
            </KbSecondaryButton>

            {testState === "ok" && (
              <span className="inline-flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.1em] text-[#3fa66a]">
                <span className="size-1.5 rounded-full bg-[#3fa66a]" />
                Connected · 142 ms
              </span>
            )}
            {testState === "fail" && (
              <span className="font-mono text-[10px] uppercase tracking-[0.1em] text-accent">
                ⚠ Could not reach endpoint
              </span>
            )}

            <div className="ml-auto flex gap-2">
              <KbGhostButton size="md" onClick={onClose}>
                Cancel
              </KbGhostButton>
              <Button
                onClick={add}
                disabled={!canAdd}
                size="sm"
                className="rounded-none"
              >
                Add backend
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

interface SectionLabelProps {
  step: string;
  label: string;
}

function SectionLabel({ step, label }: SectionLabelProps) {
  return (
    <div className="flex items-baseline gap-2 border-b border-line pb-1.5">
      <span className="font-mono text-[9px] font-bold tracking-[0.14em] text-accent">
        {step}
      </span>
      <span className="font-mono text-[10px] font-semibold uppercase tracking-[0.14em] text-ink-2">
        {label}
      </span>
    </div>
  );
}
