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
  Pencil,
  Plus,
  RefreshCw,
  Server,
  ShieldCheck,
  Sparkles,
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
import {
  AiProviderForm,
  type ExistingAiProvider,
} from "@/components/kb/AiProviderForm";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  hasSessionUnlockPassphrase,
  verifySessionUnlockPassphrase,
} from "@/store/sessionLock";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
} from "@/components/kb/Onboarding/constants";

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

interface StatusData {
  data_root: string;
  database: string;
  current_workspace: string | null;
  workspaces: number;
  profiles: number;
}

const DEFAULT_BACKENDS: Backend[] = [
  {
    id: "b1",
    name: DEFAULT_BACKEND_NAME,
    url: DEFAULT_BACKEND_URL,
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
  focusSection?: "backends" | "ai" | null;
  onLock?: () => void;
  onClose: () => void;
}

export function SettingsModal({
  open,
  focusSection = null,
  onLock,
  onClose,
}: SettingsModalProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);
  const identity = useUiStore((s) => s.identity);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const navigate = useNavigate();
  const statusQuery = useDaemon<StatusData>("status", undefined, {
    enabled: open,
  });
  const status =
    statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const deleteWorkspace = useDaemonMutation("ui.workspace.delete");
  const backendsRef = React.useRef<HTMLDivElement | null>(null);
  const aiRef = React.useRef<HTMLDivElement | null>(null);

  const [clearClipboard, setClearClipboard] = React.useState(true);
  const [autoLockEnabled, setAutoLockEnabled] = React.useState(true);
  const [autoLockMinutes, setAutoLockMinutes] = React.useState(5);
  const [requirePassphrase, setRequirePassphrase] = React.useState(true);
  const [lockOnClose, setLockOnClose] = React.useState(true);
  const [backends, setBackends] = React.useState<Backend[]>(DEFAULT_BACKENDS);
  const [backendDialogOpen, setBackendDialogOpen] = React.useState(false);
  const [editingBackendId, setEditingBackendId] = React.useState<string | null>(
    null,
  );
  const [deleteOpen, setDeleteOpen] = React.useState(false);
  const [deletePassphrase, setDeletePassphrase] = React.useState("");
  const [deleteConfirm, setDeleteConfirm] = React.useState("");
  const [deleteError, setDeleteError] = React.useState<string | null>(null);

  const editingBackend = React.useMemo(
    () => backends.find((backend) => backend.id === editingBackendId) ?? null,
    [backends, editingBackendId],
  );

  React.useEffect(() => {
    if (!open) return;
    if (focusSection !== "backends" && focusSection !== "ai") return;

    const target = focusSection === "ai" ? aiRef : backendsRef;
    const id = window.requestAnimationFrame(() => {
      target.current?.scrollIntoView({
        block: "start",
        behavior: "smooth",
      });
    });

    return () => window.cancelAnimationFrame(id);
  }, [focusSection, open]);

  const onResetWorkspace = () => {
    const ok = window.confirm(
      "Reset workspace?\n\nThis clears your local identity and returns you to the Welcome screen. Encrypted data on disk is not touched.",
    );
    if (!ok) return;
    setIdentity(null);
    onClose();
    void navigate({ to: "/", replace: true });
  };

  const lockNow = () => {
    onClose();
    window.requestAnimationFrame(() => onLock?.());
  };

  const workspaceLabel =
    status?.current_workspace || identity?.workspace || "current workspace";

  const openDeleteWorkspace = () => {
    setDeletePassphrase("");
    setDeleteConfirm("");
    setDeleteError(null);
    setDeleteOpen(true);
  };

  const openAddBackend = () => {
    setEditingBackendId(null);
    setBackendDialogOpen(true);
  };

  const openEditBackend = (backend: Backend) => {
    setEditingBackendId(backend.id);
    setBackendDialogOpen(true);
  };

  const onSaveBackend = (backend: Backend) => {
    setBackends((prev) => {
      const existingIndex = prev.findIndex((item) => item.id === backend.id);
      if (existingIndex === -1) return [...prev, backend];

      return prev.map((item) => (item.id === backend.id ? backend : item));
    });
    setBackendDialogOpen(false);
    setEditingBackendId(null);
  };

  const onDeleteBackend = (backend: Backend) => {
    const ok = window.confirm(
      `Delete backend '${backend.name}'?\n\nWallets using this endpoint may need another backend before they can sync.`,
    );
    if (!ok) return;
    setBackends((prev) => prev.filter((item) => item.id !== backend.id));
  };

  const onDeleteWorkspace = async () => {
    setDeleteError(null);
    if (!deletePassphrase) {
      setDeleteError("Enter the database passphrase.");
      return;
    }
    if (deleteConfirm.trim() !== workspaceLabel) {
      setDeleteError(`Type ${workspaceLabel} to confirm workspace deletion.`);
      return;
    }
    if (hasSessionUnlockPassphrase()) {
      const verified = await verifySessionUnlockPassphrase(deletePassphrase);
      if (!verified) {
        setDeleteError("Passphrase did not unlock this session.");
        setDeletePassphrase("");
        return;
      }
    }

    try {
      await deleteWorkspace.mutateAsync({
        confirm: "DELETE",
        confirm_workspace: workspaceLabel,
        auth_response: { passphrase_secret: deletePassphrase },
      });
      setIdentity(null);
      onClose();
      void navigate({ to: "/", replace: true });
    } catch (error) {
      window.alert(
        error instanceof Error
          ? error.message
          : "Workspace delete failed.",
      );
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
    <DialogContent className="max-h-[92vh] w-[min(96vw,1500px)] !max-w-[min(96vw,1500px)] gap-0 overflow-hidden p-0">
        <DialogHeader className="shrink-0 border-b px-6 py-5 lg:px-8">
          <DialogTitle className="text-xl">Settings</DialogTitle>
          <DialogDescription>
            Workspace preferences, privacy controls, and local data tools.
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="h-[min(78vh,820px)]">
          <div className="flex min-w-0 flex-col gap-4 p-4 lg:p-6">
            <div className="grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-2">
              <Card className="min-w-0">
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

              <Card className="min-w-0">
                <CardHeader>
                  <CardTitle className="text-base">Display currency</CardTitle>
                  <CardDescription>
                    Choose how balances and reports are shown across the app.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex max-w-md items-center justify-between gap-3 rounded-md border bg-background px-4 py-3">
                    <span
                      className={cn(
                        "text-sm font-medium",
                        currency === "eur"
                          ? "text-foreground"
                          : "text-muted-foreground",
                      )}
                    >
                      € Euro
                    </span>
                    <Switch
                      checked={currency === "btc"}
                      onCheckedChange={(checked) =>
                        setCurrency(checked ? "btc" : "eur")
                      }
                      aria-label="Display balances in Bitcoin"
                    />
                    <span
                      className={cn(
                        "text-sm font-medium",
                        currency === "btc"
                          ? "text-foreground"
                          : "text-muted-foreground",
                      )}
                    >
                      ₿ Bitcoin
                    </span>
                  </div>
                </CardContent>
              </Card>
            </div>

            <div className="grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
              <Card className="min-w-0">
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
                          variant={
                            autoLockMinutes === m ? "default" : "outline"
                          }
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
                </CardContent>
              </Card>

              <Card className="min-w-0 border-primary/15 bg-muted/20">
                <CardHeader>
                  <CardTitle className="flex items-center gap-2 text-base">
                    <KeyRound className="size-4" aria-hidden="true" />
                    Security boundary
                  </CardTitle>
                  <CardDescription>
                    What the local lock does and does not protect.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <p className="m-0 text-sm leading-6 text-muted-foreground">
                    This app lock protects against casual access in the current
                    UI session. SQLCipher plus native fd unlock is the hard
                    at-rest boundary.
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      onClick={lockNow}
                    >
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
            </div>

            <Card className="min-w-0">
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
                    <Label htmlFor="settings-data-root">Data root</Label>
                    <Input
                      id="settings-data-root"
                      readOnly
                      value={status?.data_root ?? "loading..."}
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="settings-db-path">Database</Label>
                    <Input
                      id="settings-db-path"
                      readOnly
                      value={status?.database ?? "loading..."}
                    />
                  </div>
                </div>
              </CardContent>
            </Card>

            <div ref={backendsRef} className="min-w-0">
              <Card>
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
                  <Button type="button" size="sm" onClick={openAddBackend}>
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
                          <TableHead className="text-right">Actions</TableHead>
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
                            <TableCell className="text-right">
                              <div className="flex justify-end gap-1">
                                <Button
                                  type="button"
                                  size="icon-sm"
                                  variant="ghost"
                                  aria-label={`Edit ${backend.name}`}
                                  onClick={() => openEditBackend(backend)}
                                >
                                  <Pencil
                                    className="size-3.5"
                                    aria-hidden="true"
                                  />
                                </Button>
                                <Button
                                  type="button"
                                  size="icon-sm"
                                  variant="ghost"
                                  aria-label={`Delete ${backend.name}`}
                                  onClick={() => onDeleteBackend(backend)}
                                >
                                  <Trash2
                                    className="size-3.5"
                                    aria-hidden="true"
                                  />
                                </Button>
                              </div>
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                </CardContent>
              </Card>
            </div>

            <div ref={aiRef} className="min-w-0">
              <AiProvidersCard />
            </div>

            <Card className="min-w-0 border-destructive/30">
              <CardHeader>
                <div className="space-y-1">
                  <CardTitle className="flex items-center gap-2 text-base text-destructive">
                    <Trash2 className="size-4" aria-hidden="true" />
                    Danger zone
                  </CardTitle>
                  <CardDescription>
                    Reset the Welcome gate or delete the current local workspace.
                  </CardDescription>
                </div>
              </CardHeader>
              <CardContent className="grid gap-3">
                <div className="flex flex-col gap-3 rounded-lg border p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <p className="text-sm font-medium">Reset Welcome state</p>
                    <p className="text-sm text-muted-foreground">
                      Clear only the local UI identity and return to onboarding.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="shrink-0"
                    onClick={onResetWorkspace}
                  >
                    Reset workspace
                  </Button>
                </div>
                <div className="flex flex-col gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <p className="text-sm font-medium text-destructive">
                      Delete workspace
                    </p>
                    <p className="text-sm text-muted-foreground">
                      Remove the current workspace records from the local database.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="destructive"
                    className="shrink-0"
                    disabled={deleteWorkspace.isPending}
                    onClick={openDeleteWorkspace}
                  >
                    Delete workspace
                  </Button>
                </div>
              </CardContent>
            </Card>
          </div>
        </ScrollArea>

        <DialogFooter className="shrink-0 border-t bg-background px-6 py-4 lg:px-8">
          <Button type="button" variant="outline" onClick={onClose}>
            Done
          </Button>
        </DialogFooter>

        <BackendModal
          open={backendDialogOpen}
          initial={editingBackend}
          onClose={() => {
            setBackendDialogOpen(false);
            setEditingBackendId(null);
          }}
          onSave={onSaveBackend}
        />
        <Dialog
          open={deleteOpen}
          onOpenChange={(next) => {
            if (!next) {
              setDeleteOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Delete workspace</DialogTitle>
              <DialogDescription>
                This removes {workspaceLabel} from the local Kassiber database.
                Enter the database passphrase and the workspace name to continue.
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onDeleteWorkspace();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="delete-passphrase">Passphrase</Label>
                <Input
                  id="delete-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={deletePassphrase}
                  onChange={(event) => setDeletePassphrase(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="delete-confirm">Workspace name</Label>
                <Input
                  id="delete-confirm"
                  value={deleteConfirm}
                  placeholder={workspaceLabel}
                  onChange={(event) => setDeleteConfirm(event.target.value)}
                />
              </div>
              {deleteError && (
                <p className="m-0 text-sm text-destructive">{deleteError}</p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setDeleteOpen(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={deleteWorkspace.isPending}
                >
                  {deleteWorkspace.isPending ? "Deleting..." : "Delete"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </DialogContent>
    </Dialog>
  );
}

interface AiProviderRow {
  name: string;
  base_url: string;
  kind: "local" | "remote" | "tee";
  default_model?: string | null;
  notes?: string | null;
  has_api_key: boolean;
  is_default: boolean;
  acknowledged_at?: string | null;
}

interface AiProvidersListData {
  providers: AiProviderRow[];
  default: string | null;
}

const AI_KIND_BADGE: Record<AiProviderRow["kind"], string> = {
  local: "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  remote: "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  tee: "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
};

function AiProvidersCard() {
  const providersQuery = useDaemon<AiProvidersListData>("ai.providers.list");
  const data = React.useMemo<AiProvidersListData>(
    () =>
      providersQuery.data?.kind === "ai.providers.list" &&
      providersQuery.data.data
        ? providersQuery.data.data
        : { providers: [], default: null },
    [providersQuery.data],
  );
  const setDefault = useDaemonMutation("ai.providers.set_default");
  const clearDefault = useDaemonMutation("ai.providers.clear_default");
  const deleteProvider = useDaemonMutation("ai.providers.delete");
  const [editingName, setEditingName] = React.useState<string | null>(null);
  const [addOpen, setAddOpen] = React.useState(false);

  const editingProvider = React.useMemo<ExistingAiProvider | null>(() => {
    if (!editingName) return null;
    const row = data.providers.find((p) => p.name === editingName);
    if (!row) return null;
    return {
      name: row.name,
      base_url: row.base_url,
      default_model: row.default_model ?? undefined,
      kind: row.kind,
      notes: row.notes ?? undefined,
      has_api_key: row.has_api_key,
      acknowledged_at: row.acknowledged_at ?? null,
    };
  }, [data, editingName]);

  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <CardTitle className="flex items-center gap-2 text-base">
            <Sparkles className="size-4" aria-hidden="true" />
            AI providers
          </CardTitle>
          <CardDescription>
            OpenAI-compatible endpoints for the in-app assistant. Local Ollama
            runs without a key; remote providers see prompt content.
          </CardDescription>
        </div>
        <Button type="button" size="sm" onClick={() => setAddOpen(true)}>
          <Plus className="size-4" aria-hidden="true" />
          Add provider
        </Button>
      </CardHeader>
      <CardContent>
        <div className="overflow-x-auto rounded-md border">
          <Table>
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead>Provider</TableHead>
                <TableHead>Posture</TableHead>
                <TableHead>Default model</TableHead>
                <TableHead>Auth</TableHead>
                <TableHead className="text-right">Default</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.providers.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="text-center text-sm text-muted-foreground"
                  >
                    No providers configured.
                  </TableCell>
                </TableRow>
              ) : (
                data.providers.map((row) => (
                  <TableRow key={row.name}>
                    <TableCell className="min-w-[220px]">
                      <div className="font-medium">{row.name}</div>
                      <div className="max-w-[320px] truncate text-xs text-muted-foreground">
                        {row.base_url}
                      </div>
                    </TableCell>
                    <TableCell>
                      <span
                        className={cn(
                          "inline-flex rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide",
                          AI_KIND_BADGE[row.kind],
                        )}
                      >
                        {row.kind === "tee" ? "TEE" : row.kind}
                      </span>
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {row.default_model ?? "-"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.has_api_key ? "Bearer" : "none"}
                    </TableCell>
                    <TableCell className="text-right">
                      {row.is_default ? (
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          onClick={() => clearDefault.mutate(undefined)}
                          disabled={clearDefault.isPending}
                        >
                          Clear default
                        </Button>
                      ) : (
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          onClick={() => setDefault.mutate({ name: row.name })}
                          disabled={setDefault.isPending}
                        >
                          Set default
                        </Button>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          type="button"
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Edit ${row.name}`}
                          onClick={() => setEditingName(row.name)}
                        >
                          <Pencil className="size-3.5" aria-hidden="true" />
                        </Button>
                        <Button
                          type="button"
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Delete ${row.name}`}
                          disabled={row.is_default || deleteProvider.isPending}
                          onClick={() => {
                            const ok = window.confirm(
                              `Delete AI provider '${row.name}'? Cannot be undone.`,
                            );
                            if (!ok) return;
                            deleteProvider.mutate({ name: row.name });
                          }}
                        >
                          <Trash2 className="size-3.5" aria-hidden="true" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </CardContent>

      <AiProviderForm
        open={addOpen}
        initial={null}
        onClose={() => setAddOpen(false)}
        onSaved={() => setAddOpen(false)}
      />
      <AiProviderForm
        open={Boolean(editingProvider)}
        initial={editingProvider}
        onClose={() => setEditingName(null)}
        onSaved={() => setEditingName(null)}
      />
    </Card>
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
        name: DEFAULT_BACKEND_NAME,
        url: DEFAULT_BACKEND_URL,
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

interface BackendModalProps {
  open: boolean;
  initial: Backend | null;
  onClose: () => void;
  onSave: (backend: Backend) => void;
}

function BackendModal({
  open,
  initial,
  onClose,
  onSave,
}: BackendModalProps) {
  const [typeId, setTypeId] = React.useState("btc");
  const [presetId, setPresetId] = React.useState("mempool");
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState(DEFAULT_BACKEND_URL);
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
  const isEditing = Boolean(initial);

  React.useEffect(() => {
    if (!open) return;
    if (initial) {
      const initialType =
        BACKEND_TYPES.find((candidate) => candidate.net === initial.net) ??
        BACKEND_TYPES[0];
      setTypeId(initialType.id);
      setPresetId("custom");
      setName(initial.name);
      setUrl(initial.url);
      setAuth(initial.auth);
      setAuthVal("");
      setAuthVal2("");
      setTestState(initial.on ? "ok" : "idle");
      return;
    }

    setTypeId("btc");
    setPresetId("mempool");
    setName(DEFAULT_BACKEND_NAME);
    setUrl(DEFAULT_BACKEND_URL);
    setAuth("none");
    setAuthVal("");
    setAuthVal2("");
    setTestState("idle");
  }, [initial, open]);

  React.useEffect(() => {
    if (!open) return;
    if (initial) return;
    if (preset) {
      setUrl(preset.url);
      setName(preset.name);
    } else if (presetId === "custom") {
      setUrl("");
      setName("");
    }
    setTestState("idle");
  }, [initial, open, preset, presetId]);

  const onPickType = (id: string) => {
    setTypeId(id);
    if (initial) {
      setPresetId("custom");
      return;
    }
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
  const save = () => {
    if (!canAdd) return;
    const normalizedUrl = url.trim();
    const urlChanged = Boolean(initial && normalizedUrl !== initial.url);
    onSave({
      id: initial?.id ?? "b" + Date.now(),
      name: name.trim(),
      url: normalizedUrl,
      net: type.net,
      health:
        testState === "ok"
          ? initial
            ? "just checked - ok"
            : "just added - ok"
          : testState === "fail"
            ? "-"
            : urlChanged
              ? "-"
              : (initial?.health ?? "-"),
      on:
        testState === "ok"
          ? true
          : testState === "fail"
            ? false
            : urlChanged
              ? false
              : (initial?.on ?? false),
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
          <DialogTitle>{isEditing ? "Edit backend" : "Add backend"}</DialogTitle>
          <DialogDescription>
            {isEditing
              ? "Update this endpoint's label, network, URL, and auth mode."
              : "Connect a Bitcoin, Lightning, Liquid, or price backend."}
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

            {!isEditing && type.presets.length > 0 && (
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
          <Button type="button" disabled={!canAdd} onClick={save}>
            {isEditing ? "Save backend" : "Add backend"}
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
