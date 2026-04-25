/**
 * SettingsModal - workspace-wide preferences.
 *
 * Most controls are local UI state until the daemon-backed settings surface
 * lands. Hide-sensitive data is wired to the shared UI store.
 */
import * as React from "react";
import {
  Database,
  Download,
  FileInput,
  KeyRound,
  Lock,
  Plus,
  RefreshCw,
  Server,
  ShieldCheck,
  Trash2,
  Upload,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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
    health: "#893,014 - 2m",
    on: true,
    auth: "none",
  },
  {
    id: "b2",
    name: "local electrs",
    url: "tcp://127.0.0.1:50001",
    net: "BTC",
    health: "-",
    on: false,
    auth: "none",
  },
  {
    id: "b3",
    name: "Blockstream Liquid",
    url: "https://blockstream.info/liquid/api",
    net: "LIQUID",
    health: "-",
    on: false,
    auth: "none",
  },
  {
    id: "b4",
    name: "CoinGecko",
    url: "https://api.coingecko.com/api/v3",
    net: "FX",
    health: "EUR 71,420 - 14s",
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
  const setIdentity = useUiStore((s) => s.setIdentity);
  const navigate = useNavigate();

  const [clearClipboard, setClearClipboard] = React.useState(true);
  const [autoLockEnabled, setAutoLockEnabled] = React.useState(true);
  const [autoLockMinutes, setAutoLockMinutes] = React.useState(5);
  const [requirePassphrase, setRequirePassphrase] = React.useState(true);
  const [lockOnClose, setLockOnClose] = React.useState(true);
  const [backends, setBackends] = React.useState<Backend[]>(DEFAULT_BACKENDS);
  const [addOpen, setAddOpen] = React.useState(false);

  const onResetWorkspace = () => {
    const ok = window.confirm(
      "Reset workspace?\n\nThis clears your local identity and returns you to the Welcome screen. Encrypted data on disk is not touched.",
    );
    if (!ok) return;
    setIdentity(null);
    onClose();
    navigate({ to: "/" });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="max-h-[88vh] w-full max-w-[900px] gap-0 overflow-hidden p-0 sm:max-w-[900px]">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>
            Workspace preferences, privacy controls, and local data tools.
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(88vh-132px)]">
          <div className="grid gap-4 p-4 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <ShieldCheck className="size-4" aria-hidden="true" />
                  Privacy
                </CardTitle>
                <CardDescription>
                  Controls for sensitive values shown inside the app.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <SettingsSwitchRow
                  label="Hide sensitive data"
                  description="Blur balances, addresses, and amounts throughout the UI."
                  checked={hideSensitive}
                  onCheckedChange={setHideSensitive}
                />
                <SettingsSwitchRow
                  label="Clear clipboard after 30s"
                  description="Auto-clear copied addresses and keys."
                  checked={clearClipboard}
                  onCheckedChange={setClearClipboard}
                />
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <Lock className="size-4" aria-hidden="true" />
                  App lock
                </CardTitle>
                <CardDescription>
                  Local lock behavior for decrypted workspace state.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <SettingsSwitchRow
                  label="Auto-lock when idle"
                  description="Require passphrase after inactivity."
                  checked={autoLockEnabled}
                  onCheckedChange={setAutoLockEnabled}
                />
                <div
                  className={cn(
                    "space-y-2",
                    !autoLockEnabled && "pointer-events-none opacity-50",
                  )}
                >
                  <Label>Idle timeout</Label>
                  <div className="flex flex-wrap gap-2">
                    {[1, 5, 15, 30, 60].map((m) => (
                      <Button
                        key={m}
                        type="button"
                        variant={autoLockMinutes === m ? "default" : "outline"}
                        size="sm"
                        onClick={() => setAutoLockMinutes(m)}
                      >
                        {m}m
                      </Button>
                    ))}
                  </div>
                </div>
                <SettingsSwitchRow
                  label="Require passphrase on launch"
                  description="Prompt every time Kassiber opens."
                  checked={requirePassphrase}
                  onCheckedChange={setRequirePassphrase}
                />
                <SettingsSwitchRow
                  label="Lock on window close"
                  description="Clear in-memory decrypted state when the app window closes."
                  checked={lockOnClose}
                  onCheckedChange={setLockOnClose}
                />
                <div className="flex flex-wrap gap-2 pt-1">
                  <Button type="button" size="sm" variant="outline">
                    <Lock className="size-4" aria-hidden="true" />
                    Lock now
                  </Button>
                  <Button type="button" size="sm" variant="ghost">
                    <KeyRound className="size-4" aria-hidden="true" />
                    Change passphrase
                  </Button>
                </div>
              </CardContent>
            </Card>

            <Card className="lg:col-span-2">
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <Database className="size-4" aria-hidden="true" />
                  Data
                </CardTitle>
                <CardDescription>
                  Backup, restore, labels, imports, and local database status.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid gap-2 sm:grid-cols-3">
                  <Button type="button" variant="outline" className="justify-start">
                    <Download className="size-4" aria-hidden="true" />
                    Backup
                  </Button>
                  <Button type="button" variant="outline" className="justify-start">
                    <Upload className="size-4" aria-hidden="true" />
                    Restore
                  </Button>
                  <Button type="button" variant="outline" className="justify-start">
                    <FileInput className="size-4" aria-hidden="true" />
                    Logs
                  </Button>
                </div>

                <Separator />

                <div className="grid gap-2 sm:grid-cols-3">
                  <Button type="button" variant="secondary" className="justify-start">
                    Import BIP-329
                  </Button>
                  <Button type="button" variant="secondary" className="justify-start">
                    Export BIP-329
                  </Button>
                  <Button type="button" variant="secondary" className="justify-start">
                    Import CSV
                  </Button>
                </div>

                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-1.5">
                    <Label htmlFor="settings-db-path">Database</Label>
                    <Input
                      id="settings-db-path"
                      readOnly
                      value="~/.kassiber/kassiber.db"
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="settings-last-backup">Last backup</Label>
                    <Input
                      id="settings-last-backup"
                      readOnly
                      value="2026-04-17 23:02 - backup_2026-04-17.tar.zst"
                    />
                  </div>
                </div>
              </CardContent>
            </Card>

            <Card className="lg:col-span-2">
              <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <CardTitle className="flex items-center gap-2 text-base">
                    <Server className="size-4" aria-hidden="true" />
                    Sync backends
                  </CardTitle>
                  <CardDescription>
                    Local node, indexer, and rate endpoints available to the workspace.
                  </CardDescription>
                </div>
                <Button type="button" size="sm" onClick={() => setAddOpen(true)}>
                  <Plus className="size-4" aria-hidden="true" />
                  Add backend
                </Button>
              </CardHeader>
              <CardContent>
                <div className="overflow-x-auto rounded-md border">
                  <Table>
                    <TableHeader>
                      <TableRow className="bg-muted/50 hover:bg-muted/50">
                        <TableHead>Backend</TableHead>
                        <TableHead>Network</TableHead>
                        <TableHead>Health</TableHead>
                        <TableHead>Auth</TableHead>
                        <TableHead className="text-right">Status</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {backends.map((backend) => (
                        <TableRow key={backend.id}>
                          <TableCell className="min-w-[240px]">
                            <div className="font-medium">{backend.name}</div>
                            <div className="max-w-[360px] truncate text-xs text-muted-foreground">
                              {backend.url}
                            </div>
                          </TableCell>
                          <TableCell>
                            <NetworkBadge net={backend.net} />
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {backend.health}
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {backend.auth}
                          </TableCell>
                          <TableCell className="text-right">
                            <StatusBadge active={backend.on} />
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              </CardContent>
            </Card>

            <Card className="border-destructive/30 lg:col-span-2">
              <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <CardTitle className="flex items-center gap-2 text-base text-destructive">
                    <Trash2 className="size-4" aria-hidden="true" />
                    Danger zone
                  </CardTitle>
                  <CardDescription>
                    Reset local identity and return to the Welcome screen.
                  </CardDescription>
                </div>
                <Button
                  type="button"
                  variant="destructive"
                  onClick={onResetWorkspace}
                >
                  Reset workspace
                </Button>
              </CardHeader>
            </Card>
          </div>
        </ScrollArea>

        <DialogFooter className="border-t px-6 py-4">
          <Button type="button" variant="outline" onClick={onClose}>
            Done
          </Button>
        </DialogFooter>

        <AddBackendModal
          open={addOpen}
          onClose={() => setAddOpen(false)}
          onAdd={(backend) => {
            setBackends((prev) => [...prev, backend]);
            setAddOpen(false);
          }}
        />
      </DialogContent>
    </Dialog>
  );
}

interface SettingsSwitchRowProps {
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}

function SettingsSwitchRow({
  label,
  description,
  checked,
  onCheckedChange,
}: SettingsSwitchRowProps) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0 space-y-1">
        <Label className="text-sm font-medium">{label}</Label>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function NetworkBadge({ net }: { net: Net }) {
  const classes: Record<Net, string> = {
    BTC: "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    LIQUID:
      "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
    LN: "border-violet-500/25 bg-violet-500/10 text-violet-700 dark:text-violet-300",
    FX: "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  };

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
        classes[net],
      )}
    >
      {net}
    </span>
  );
}

function StatusBadge({ active }: { active: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
        active
          ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : "border-border bg-muted text-muted-foreground",
      )}
    >
      {active ? "Active" : "Idle"}
    </span>
  );
}

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
      {
        id: "mempool",
        name: "mempool.space",
        url: "https://mempool.space/api",
        scheme: "REST",
      },
      {
        id: "esplora",
        name: "Blockstream Esplora",
        url: "https://blockstream.info/api",
        scheme: "REST",
      },
      {
        id: "electrum",
        name: "Electrum server",
        url: "tcp://127.0.0.1:50001",
        scheme: "Electrum",
      },
      {
        id: "core",
        name: "Bitcoin Core RPC",
        url: "http://127.0.0.1:8332",
        scheme: "RPC",
      },
    ],
  },
  {
    id: "lightning",
    label: "Lightning",
    net: "LN",
    desc: "Read channel state, invoices and forwards from an LN node.",
    presets: [
      {
        id: "lnd",
        name: "LND",
        url: "https://127.0.0.1:8080",
        scheme: "REST",
      },
      {
        id: "cln",
        name: "Core Lightning",
        url: "http://127.0.0.1:3010",
        scheme: "CLNREST",
      },
      {
        id: "lnbits",
        name: "LNbits",
        url: "https://your.lnbits.host",
        scheme: "REST",
      },
      {
        id: "nwc",
        name: "Nostr Wallet Connect",
        url: "nostr+walletconnect://",
        scheme: "NWC",
      },
    ],
  },
  {
    id: "liquid",
    label: "Liquid",
    net: "LIQUID",
    desc: "Read Liquid balances and sidechain activity.",
    presets: [
      {
        id: "blockstream",
        name: "Blockstream Liquid",
        url: "https://blockstream.info/liquid/api",
        scheme: "REST",
      },
      {
        id: "liquidcore",
        name: "Elements RPC",
        url: "http://127.0.0.1:7041",
        scheme: "RPC",
      },
    ],
  },
  {
    id: "fx",
    label: "Price / FX",
    net: "FX",
    desc: "BTC/EUR and other fiat reference rates.",
    presets: [
      {
        id: "coingecko",
        name: "CoinGecko",
        url: "https://api.coingecko.com/api/v3",
        scheme: "REST",
      },
      {
        id: "kraken",
        name: "Kraken",
        url: "https://api.kraken.com/0/public",
        scheme: "REST",
      },
      {
        id: "bitstamp",
        name: "Bitstamp",
        url: "https://www.bitstamp.net/api/v2",
        scheme: "REST",
      },
    ],
  },
  {
    id: "other",
    label: "Other",
    net: "FX",
    desc: "A generic HTTP or WebSocket endpoint.",
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
    BACKEND_TYPES.find((candidate) => candidate.id === typeId) ??
    BACKEND_TYPES[0];
  const preset =
    presetId === "custom"
      ? null
      : type.presets.find((candidate) => candidate.id === presetId) ?? null;

  React.useEffect(() => {
    if (!open) return;
    setTypeId("btc");
    setPresetId("mempool");
    setName("mempool.space");
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
  }, [open, preset, presetId]);

  const onPickType = (id: string) => {
    setTypeId(id);
    const nextType = BACKEND_TYPES.find((candidate) => candidate.id === id);
    setPresetId(nextType?.presets[0]?.id ?? "custom");
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
      health: testState === "ok" ? "just added - ok" : "-",
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
      <DialogContent className="max-h-[88vh] w-full max-w-[760px] overflow-hidden p-0 sm:max-w-[760px]">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>Add backend</DialogTitle>
          <DialogDescription>
            Connect a Bitcoin, Lightning, Liquid, or price backend.
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(88vh-150px)]">
          <div className="space-y-5 p-6">
            <section className="space-y-3">
              <div>
                <Label>Backend type</Label>
                <p className="text-sm text-muted-foreground">{type.desc}</p>
              </div>
              <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
                {BACKEND_TYPES.map((backendType) => {
                  const active = backendType.id === typeId;
                  return (
                    <Button
                      key={backendType.id}
                      type="button"
                      variant={active ? "default" : "outline"}
                      className="h-auto min-h-20 flex-col items-start justify-start gap-2 whitespace-normal p-3 text-left"
                      onClick={() => onPickType(backendType.id)}
                    >
                      <NetworkBadge net={backendType.net} />
                      <span className="text-sm leading-tight font-medium">
                        {backendType.label}
                      </span>
                    </Button>
                  );
                })}
              </div>
            </section>

            {type.presets.length > 0 && (
              <section className="space-y-3">
                <Label>Preset</Label>
                <div className="flex flex-wrap gap-2">
                  {type.presets.map((backendPreset) => (
                    <Button
                      key={backendPreset.id}
                      type="button"
                      variant={
                        presetId === backendPreset.id ? "default" : "outline"
                      }
                      size="sm"
                      onClick={() => setPresetId(backendPreset.id)}
                    >
                      {backendPreset.name}
                      <span className="text-xs opacity-70">
                        {backendPreset.scheme}
                      </span>
                    </Button>
                  ))}
                  <Button
                    type="button"
                    variant={presetId === "custom" ? "default" : "outline"}
                    size="sm"
                    onClick={() => setPresetId("custom")}
                  >
                    Custom
                  </Button>
                </div>
              </section>
            )}

            <section className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="backend-name">Display name</Label>
                <Input
                  id="backend-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="My home node"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="backend-url">Endpoint URL</Label>
                <Input
                  id="backend-url"
                  value={url}
                  onChange={(event) => {
                    setUrl(event.target.value);
                    setTestState("idle");
                  }}
                  placeholder="https://..."
                />
              </div>
            </section>

            <section className="space-y-3">
              <Label>Authentication</Label>
              <div className="flex flex-wrap gap-2">
                {AUTH_MODES.map((mode) => (
                  <Button
                    key={mode.id}
                    type="button"
                    variant={auth === mode.id ? "default" : "outline"}
                    size="sm"
                    onClick={() => setAuth(mode.id)}
                  >
                    {mode.label}
                  </Button>
                ))}
              </div>
              {auth === "apikey" && (
                <SecretField
                  id="backend-api-key"
                  label="API key"
                  value={authVal}
                  onChange={setAuthVal}
                  placeholder="sk_live_..."
                />
              )}
              {auth === "bearer" && (
                <SecretField
                  id="backend-bearer"
                  label="Bearer token"
                  value={authVal}
                  onChange={setAuthVal}
                  placeholder="eyJ..."
                />
              )}
              {auth === "basic" && (
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="backend-username">Username</Label>
                    <Input
                      id="backend-username"
                      value={authVal}
                      onChange={(event) => setAuthVal(event.target.value)}
                    />
                  </div>
                  <SecretField
                    id="backend-password"
                    label="Password"
                    value={authVal2}
                    onChange={setAuthVal2}
                  />
                </div>
              )}
            </section>

            <div className="rounded-md border bg-muted/30 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={testConnection}
                  disabled={!url.trim() || testState === "testing"}
                >
                  <RefreshCw
                    className={cn(
                      "size-4",
                      testState === "testing" && "animate-spin",
                    )}
                    aria-hidden="true"
                  />
                  {testState === "testing" ? "Testing" : "Test connection"}
                </Button>
                {testState === "ok" && (
                  <span className="text-emerald-600">Connected - 142 ms</span>
                )}
                {testState === "fail" && (
                  <span className="text-destructive">
                    Could not reach endpoint
                  </span>
                )}
              </div>
            </div>
          </div>
        </ScrollArea>

        <DialogFooter className="border-t px-6 py-4">
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={!canAdd} onClick={add}>
            Add backend
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface SecretFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

function SecretField({
  id,
  label,
  value,
  onChange,
  placeholder,
}: SecretFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}
