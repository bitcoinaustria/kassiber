import * as React from "react";
import {
  CheckCircle2,
  RefreshCw,
  Server,
  Trash2,
  XCircle,
} from "lucide-react";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.png";
import liquidIcon from "@/assets/integrations/liquid.svg";
import mempoolIcon from "@/assets/integrations/mempool-space.svg";
import { Button } from "@/components/ui/button";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useDaemonMutation } from "@/daemon/client";
import type { DeferredConnectionSetup } from "@/store/ui";
import {
  CLN_PRESENCE_SENTINEL_COMMANDO_PEER,
  CLN_PRESENCE_SENTINEL_LIGHTNING_DIR,
  CLN_PRESENCE_SENTINEL_RPC_FILE,
  coreLightningBackendModeValid,
} from "@/lib/lightning";
import { backendTrustFromEndpoint, inferredInfrastructureOwnership, type InfrastructureOwnership } from "@/lib/backendTrust";
import {
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
} from "@/components/kb/Onboarding/constants";
import { cn } from "@/lib/utils";
import { SecretField } from "./SettingsControls";
import {
  brandLogoFrame,
  type Backend,
  type Net,
} from "./SettingsModel";

export function NetworkBadge({ net }: { net: Net }) {
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

export function NetworkMark({ type }: { type: SyncBackendNetwork }) {
  if (!type.icon) return <NetworkBadge net={type.net} />;
  return (
    <span
      className={cn(
        "flex size-9 shrink-0 items-center justify-center rounded-md border p-1.5",
        type.iconFrameClassName ?? brandLogoFrame,
      )}
      aria-hidden="true"
    >
      <img
        src={type.icon}
        alt=""
        className={cn("size-6 object-contain", type.iconClassName)}
      />
    </span>
  );
}

export function PresetMark({
  preset,
  net,
}: {
  preset: SyncBackendPreset;
  net: Net;
}) {
  const image =
    preset.id === "mempool"
      ? mempoolIcon
      : preset.protocol === "coreln"
        ? coreLightningIcon
        : preset.protocol === "lnd"
          ? lightningLabsIcon
          : net === "LIQUID"
            ? liquidIcon
            : preset.protocol === "esplora"
              ? bitcoinIcon
              : null;
  if (image) {
    return (
      <span
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-md border p-1.5",
          preset.protocol === "coreln"
            ? "border-neutral-600 bg-[#494120]"
            : preset.protocol === "lnd"
              ? "border-neutral-700 bg-neutral-950"
              : brandLogoFrame,
        )}
        aria-hidden="true"
      >
        <img
          src={image}
          alt=""
          className={cn(
            "size-5 object-contain",
            preset.protocol === "coreln" && "scale-150",
            net === "LIQUID" && "scale-150",
          )}
        />
      </span>
    );
  }
  return (
    <span
      className="flex size-8 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground"
      aria-hidden="true"
    >
      <Server className="size-4" />
    </span>
  );
}

export function presetDisplayName(preset: SyncBackendPreset): string {
  return preset.providerLabel ?? preset.name;
}

export function selectorButtonClass(active: boolean) {
  return cn(
    "border text-foreground shadow-xs transition-colors",
    active
      ? "border-foreground/50 bg-muted text-foreground ring-1 ring-foreground/10 hover:bg-muted/90 dark:border-white/45 dark:bg-white/[0.10] dark:text-white dark:ring-white/10 dark:hover:bg-white/[0.14]"
      : "border-border bg-background hover:border-foreground/35 hover:bg-muted dark:border-white/20 dark:bg-white/[0.04] dark:text-white dark:hover:border-white/40 dark:hover:bg-white/[0.08]",
  );
}

export interface SyncBackendPreset {
  id: string;
  name: string;
  url: string;
  protocol:
    | "esplora"
    | "electrum"
    | "bitcoinrpc"
    | "liquid-esplora"
    | "lnd"
    | "coreln";
  label: string;
  providerLabel?: string;
  publicPreset?: boolean;
  disabled?: boolean;
  status?: string;
}

export interface SyncBackendNetwork {
  id: "bitcoin" | "liquid" | "coreln" | "lnd";
  label: string;
  net: Net;
  desc: string;
  icon: string;
  iconClassName?: string;
  iconFrameClassName?: string;
  subtitle?: string;
  presets: SyncBackendPreset[];
}

export const SYNC_BACKEND_NETWORKS: SyncBackendNetwork[] = [
  {
    id: "bitcoin",
    label: "Bitcoin",
    net: "BTC",
    desc: "Backends used by Bitcoin watch-only wallets.",
    icon: bitcoinIcon,
    subtitle: "Bitcoin",
    presets: [
      {
        id: "mempool",
        name: DEFAULT_BACKEND_NAME,
        url: DEFAULT_BACKEND_URL,
        protocol: "esplora",
        label: "Explorer API",
        providerLabel: "mempool.bitcoin-austria.at",
      },
      {
        id: "electrum",
        name: "Bitcoin Austria Fulcrum",
        url: "ssl://index.bitcoin-austria.at:50002",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
        providerLabel: "Bitcoin Austria",
      },
      {
        id: "core",
        name: "Bitcoin Core RPC",
        url: "http://127.0.0.1:8332",
        protocol: "bitcoinrpc",
        label: "Bitcoin Core RPC",
        publicPreset: false,
      },
    ],
  },
  {
    id: "coreln",
    label: "Core-LN",
    net: "LN",
    desc: "Read-only Core Lightning node accounting sync.",
    icon: coreLightningIcon,
    iconClassName: "scale-150",
    iconFrameClassName: "border-neutral-600 bg-[#494120]",
    subtitle: "Lightning",
    presets: [
      {
        id: "core-lightning",
        name: "Core Lightning read-only",
        url: "cln://commando",
        protocol: "coreln",
        label: "Commando rune",
      },
    ],
  },
  {
    id: "liquid",
    label: "Liquid",
    net: "LIQUID",
    desc: "Backends used by Liquid watch-only wallets.",
    icon: liquidIcon,
    iconClassName: "scale-150",
    subtitle: "Liquid",
    presets: [
      {
        id: "liquid-electrum",
        name: "Liquid Electrum",
        url: "ssl://liquid.example:50002",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
        publicPreset: false,
      },
      {
        id: "liquid-network",
        name: "Liquid Network",
        url: "https://liquid.network/api",
        protocol: "liquid-esplora",
        label: "Explorer API",
        providerLabel: "Liquid Network",
      },
    ],
  },
  {
    id: "lnd",
    label: "LND",
    net: "LN",
    desc: "Read-only Lightning node history for profitability reports.",
    icon: lightningLabsIcon,
    iconFrameClassName: "border-neutral-700 bg-neutral-950",
    subtitle: "Lightning",
    presets: [
      {
        id: "lnd",
        name: "LND",
        url: "https://127.0.0.1:8080",
        protocol: "lnd",
        label: "LND REST",
      },
    ],
  },
];

export function scopedBackendTypes(
  initialTypeId?: SyncBackendNetwork["id"],
): SyncBackendNetwork[] {
  if (initialTypeId === "bitcoin") {
    return SYNC_BACKEND_NETWORKS.filter((candidate) => candidate.id === "bitcoin");
  }
  if (initialTypeId === "liquid") {
    return SYNC_BACKEND_NETWORKS.filter((candidate) => candidate.id === "liquid");
  }
  if (initialTypeId === "coreln" || initialTypeId === "lnd") {
    return SYNC_BACKEND_NETWORKS.filter(
      (candidate) => candidate.id === "coreln" || candidate.id === "lnd",
    );
  }
  return SYNC_BACKEND_NETWORKS;
}

export function backendModalCopy({
  isEditing,
  typeId,
  scopedTypes,
}: {
  isEditing: boolean;
  typeId: SyncBackendNetwork["id"];
  scopedTypes: SyncBackendNetwork[];
}): { title: string; description: string; selectorLabel: string } {
  if (isEditing) {
    return {
      title: "Edit sync backend",
      description: "Update this wallet-refresh endpoint.",
      selectorLabel: "Network",
    };
  }
  if (scopedTypes.length === 1) {
    if (typeId === "bitcoin") {
      return {
        title: "Add Bitcoin backend",
        description:
          "Connect a Bitcoin indexer or node used by watch-only wallets.",
        selectorLabel: "Network",
      };
    }
    if (typeId === "liquid") {
      return {
        title: "Add Liquid backend",
        description:
          "Connect a Liquid indexer used by Liquid watch-only wallets.",
        selectorLabel: "Network",
      };
    }
  }
  if (scopedTypes.every((candidate) => candidate.net === "LN")) {
    return {
      title: "Add Lightning node",
      description:
        "Connect a read-only Lightning node for accounting and profitability reports.",
      selectorLabel: "Node",
    };
  }
  return {
    title: "Add sync backend",
    description: "Connect a Bitcoin, Liquid, or Lightning backend.",
    selectorLabel: "Network",
  };
}

export const AUTH_MODES: Array<{ id: string; label: string }> = [
  { id: "none", label: "None" },
  { id: "apikey", label: "API key" },
  { id: "basic", label: "User + pass" },
  { id: "bearer", label: "Bearer token" },
];

export function normalizedBackendKind(kind: string | null | undefined): string {
  return (kind ?? "").toLowerCase().replace(/-/g, "");
}

export function backendTypeIdForConnectionSetup(
  intent: DeferredConnectionSetup | null,
): SyncBackendNetwork["id"] | undefined {
  const kind = normalizedBackendKind(intent?.backendKind);
  if (kind === "coreln") return "coreln";
  if (kind === "lnd") return "lnd";
  if (intent?.sourceId === "core-ln") return "coreln";
  if (intent?.sourceId === "lnd") return "lnd";
  return undefined;
}

export type TestState = "idle" | "testing" | "ok" | "fail";
export type BackendSourceMode = "preset" | "custom";

export interface ElectrumEndpointParts {
  host: string;
  port: string;
  useSsl: boolean;
}

export function parseElectrumEndpoint(raw: string): ElectrumEndpointParts {
  const candidate = raw.includes("://") ? raw : `ssl://${raw}`;
  try {
    const parsed = new URL(candidate);
    const useSsl = parsed.protocol !== "tcp:";
    return {
      host: parsed.hostname,
      port: parsed.port || (useSsl ? "50002" : "50001"),
      useSsl,
    };
  } catch {
    return { host: "", port: "50002", useSsl: true };
  }
}

export function buildElectrumUrl({ host, port, useSsl }: ElectrumEndpointParts): string {
  const trimmedHost = host.trim();
  const trimmedPort = port.trim();
  if (!trimmedHost || !trimmedPort) return "";
  return `${useSsl ? "ssl" : "tcp"}://${trimmedHost}:${trimmedPort}`;
}

export function customBackendName(
  type: SyncBackendNetwork,
  preset: SyncBackendPreset | null,
): string {
  if (type.net === "LIQUID") return "My Liquid backend";
  if (type.net === "BTC") return "My Bitcoin backend";
  return preset?.name ?? "My backend";
}

export function applyCustomEndpointDefaults(
  preset: SyncBackendPreset | null,
  {
    setUrl,
    setElectrumHost,
    setElectrumPort,
    setElectrumUseSsl,
  }: {
    setUrl: (value: string) => void;
    setElectrumHost: (value: string) => void;
    setElectrumPort: (value: string) => void;
    setElectrumUseSsl: (value: boolean) => void;
  },
) {
  if (preset?.protocol === "electrum") {
    setUrl("");
    setElectrumHost("");
    setElectrumPort("50002");
    setElectrumUseSsl(true);
    return;
  }
  setUrl("");
}

export function randomPreset(type: SyncBackendNetwork): SyncBackendPreset | null {
  const candidates = publicBackendPresets(type);
  if (candidates.length === 0) return null;
  const cryptoApi = globalThis.crypto;
  if (cryptoApi?.getRandomValues) {
    const values = new Uint32Array(1);
    cryptoApi.getRandomValues(values);
    return candidates[values[0] % candidates.length];
  }
  return candidates[Math.floor(Math.random() * candidates.length)];
}

export function publicBackendPresets(type: SyncBackendNetwork): SyncBackendPreset[] {
  return type.presets.filter(
    (candidate) => candidate.publicPreset !== false && !candidate.disabled,
  );
}

export interface SyncBackendSettingsModalProps {
  open: boolean;
  initial: Backend | null;
  initialTypeId?: SyncBackendNetwork["id"];
  onClose: () => void;
  onDelete?: (backend: Backend) => void | Promise<void>;
  onSave: (backend: Backend) => void | Promise<void>;
}

export function SyncBackendSettingsModal({
  open,
  initial,
  initialTypeId,
  onClose,
  onDelete,
  onSave,
}: SyncBackendSettingsModalProps) {
  const testElectrum = useDaemonMutation<{
    ok: boolean;
    logs: string[];
  }>("ui.backends.electrum.test");
  const testHttp = useDaemonMutation<{
    ok: boolean;
    logs: string[];
    status?: number;
  }>("ui.backends.http.test");
  const [typeId, setTypeId] = React.useState<SyncBackendNetwork["id"]>("bitcoin");
  const [backendSource, setBackendSource] =
    React.useState<BackendSourceMode>("preset");
  const [presetId, setPresetId] = React.useState("mempool");
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState(DEFAULT_BACKEND_URL);
  const [auth, setAuth] = React.useState("none");
  const [authVal, setAuthVal] = React.useState("");
  const [authVal2, setAuthVal2] = React.useState("");
  const [electrumHost, setElectrumHost] = React.useState("");
  const [electrumPort, setElectrumPort] = React.useState("50002");
  const [electrumUseSsl, setElectrumUseSsl] = React.useState(true);
  const [trustSsl, setTrustSsl] = React.useState(false);
  const [infrastructureOwner, setInfrastructureOwner] =
    React.useState<InfrastructureOwnership>("third_party");
  const [certificate, setCertificate] = React.useState("");
  const [useProxy, setUseProxy] = React.useState(false);
  const [proxyHost, setProxyHost] = React.useState("");
  const [proxyPort, setProxyPort] = React.useState("");
  const [commandoPeerId, setCommandoPeerId] = React.useState("");
  const [lightningCli, setLightningCli] = React.useState("");
  const [lightningDir, setLightningDir] = React.useState("");
  const [rpcFile, setRpcFile] = React.useState("");
  const [testState, setTestState] = React.useState<TestState>("idle");
  const [testLog, setTestLog] = React.useState("");
  const [saveState, setSaveState] = React.useState<"idle" | "saving">("idle");

  const type =
    SYNC_BACKEND_NETWORKS.find((candidate) => candidate.id === typeId) ??
    SYNC_BACKEND_NETWORKS[0];
  const preset =
    presetId === "custom"
      ? null
      : type.presets.find((candidate) => candidate.id === presetId) ?? null;
  const isEditing = Boolean(initial);
  const scopedTypes = React.useMemo(
    () => (isEditing ? SYNC_BACKEND_NETWORKS : scopedBackendTypes(initialTypeId)),
    [initialTypeId, isEditing],
  );
  const modalCopy = backendModalCopy({
    isEditing,
    typeId,
    scopedTypes,
  });
  const publicPresets = React.useMemo(() => publicBackendPresets(type), [type]);
  const showTypePicker = !isEditing && scopedTypes.length > 1;
  const showSourcePicker = !isEditing && type.net !== "LN";
  const showPresetPicker =
    !isEditing && publicPresets.length > 0 && backendSource === "preset";
  const showCustomProtocolPicker =
    !isEditing && type.presets.length > 1 && backendSource === "custom";
  const isCoreLightning =
    preset?.protocol === "coreln" || initial?.kind === "coreln";
  const isElectrum = preset?.protocol === "electrum";
  const isLnd = preset?.protocol === "lnd" || initial?.kind === "lnd";
  const showAuth = preset?.protocol === "bitcoinrpc" || isLnd;
  const showElectrumEndpointParts = isElectrum;
  const effectiveUrl = showElectrumEndpointParts
    ? buildElectrumUrl({
        host: electrumHost,
        port: electrumPort,
        useSsl: electrumUseSsl,
      })
    : isCoreLightning
      ? url.trim() || "cln://commando"
      : url.trim();
  const selectedBackendKind =
    preset?.protocol ??
    initial?.kind ??
    (type.net === "LIQUID"
      ? "liquid-esplora"
      : type.net === "LN"
        ? "lnd"
        : "esplora");
  const selectedKindIsExplorerApi =
    selectedBackendKind === "esplora" ||
    selectedBackendKind === "liquid-esplora";
  const effectiveInfrastructureOwner =
    type.net === "LN"
      ? undefined
      : infrastructureOwner;
  const connectionTrust = backendTrustFromEndpoint(
    effectiveUrl,
    showElectrumEndpointParts && useProxy && Boolean(proxyHost.trim()),
    effectiveInfrastructureOwner,
  );
  const ConnectionTrustIcon = connectionTrust.icon;

  React.useEffect(() => {
    if (!open) return;
    if (initial) {
      const parsedElectrum = parseElectrumEndpoint(initial.url);
      const initialKind = normalizedBackendKind(initial.kind);
      const initialType =
        SYNC_BACKEND_NETWORKS.find((candidate) =>
          candidate.presets.some(
            (preset) => normalizedBackendKind(preset.protocol) === initialKind,
          ),
        ) ??
        SYNC_BACKEND_NETWORKS.find((candidate) => candidate.net === initial.net) ??
        SYNC_BACKEND_NETWORKS[0];
      const initialPreset =
        initialType.presets.find((candidate) => candidate.url === initial.url) ??
        (initial.url.match(/^(ssl|tcp):\/\//i)
          ? initialType.presets.find((candidate) => candidate.protocol === "electrum")
          : null);
      setTypeId(initialType.id);
      setBackendSource("custom");
      setPresetId(initialPreset?.id ?? "custom");
      setName(initial.name);
      setUrl(initial.url);
      setAuth(initial.auth);
      setAuthVal("");
      setAuthVal2("");
      setCommandoPeerId(
        initial.commandoPeerId === CLN_PRESENCE_SENTINEL_COMMANDO_PEER
          ? ""
          : initial.commandoPeerId ?? "",
      );
      setLightningCli(initial.lightningCli ?? "");
      setLightningDir(
        initial.lightningDir === CLN_PRESENCE_SENTINEL_LIGHTNING_DIR
          ? ""
          : initial.lightningDir ?? "",
      );
      setRpcFile(
        initial.rpcFile === CLN_PRESENCE_SENTINEL_RPC_FILE
          ? ""
          : initial.rpcFile ?? "",
      );
      setElectrumHost(parsedElectrum.host);
      setElectrumPort(parsedElectrum.port);
      setElectrumUseSsl(parsedElectrum.useSsl);
      setTrustSsl(Boolean(initial.trustSsl));
      setInfrastructureOwner(
        initial.infrastructureOwner ??
          inferredInfrastructureOwnership(initial.url),
      );
      setCertificate(initial.certificate ?? "");
      setUseProxy(Boolean(initial.proxy));
      setProxyHost(initial.proxy?.host ?? "");
      setProxyPort(initial.proxy?.port ?? "");
      setTestState(initial.on ? "ok" : "idle");
      setTestLog("");
      setSaveState("idle");
      return;
    }

    const nextType =
      scopedTypes.find((candidate) => candidate.id === initialTypeId) ??
      scopedTypes[0] ??
      SYNC_BACKEND_NETWORKS[0];
    const nextPreset = randomPreset(nextType);
    setTypeId(nextType.id);
    setBackendSource(nextType.net === "LN" ? "custom" : "preset");
    setPresetId(nextPreset?.id ?? "custom");
    setName(nextPreset?.name ?? DEFAULT_BACKEND_NAME);
    setUrl(nextPreset?.url ?? DEFAULT_BACKEND_URL);
    setAuth("none");
    setAuthVal("");
    setAuthVal2("");
    setElectrumHost("index.bitcoin-austria.at");
    setElectrumPort("50002");
    setElectrumUseSsl(true);
    setTrustSsl(false);
    setInfrastructureOwner(
      inferredInfrastructureOwnership(nextPreset?.url ?? DEFAULT_BACKEND_URL),
    );
    setCertificate("");
    setUseProxy(false);
    setProxyHost("");
    setProxyPort("");
    setTestState("idle");
    setTestLog("");
    setSaveState("idle");
  }, [initial, initialTypeId, open, scopedTypes]);

  React.useEffect(() => {
    if (!open) return;
    if (initial) return;
    if (preset) {
      if (backendSource === "preset" || type.net === "LN") {
        setUrl(preset.url);
        setName(preset.name);
        setInfrastructureOwner(inferredInfrastructureOwnership(preset.url));
      } else {
        setName(customBackendName(type, preset));
        applyCustomEndpointDefaults(preset, {
          setUrl,
          setElectrumHost,
          setElectrumPort,
          setElectrumUseSsl,
        });
        setInfrastructureOwner(inferredInfrastructureOwnership(preset.url));
      }
      setAuth(preset.protocol === "lnd" ? "apikey" : "none");
      if (backendSource === "preset" && preset.protocol === "electrum") {
        const parsed = parseElectrumEndpoint(preset.url);
        setElectrumHost(parsed.host);
        setElectrumPort(parsed.port);
        setElectrumUseSsl(parsed.useSsl);
      }
    } else if (presetId === "custom") {
      setUrl("");
      setName("");
      setAuth("none");
      setInfrastructureOwner("third_party");
    }
    setAuthVal("");
    setAuthVal2("");
    setCommandoPeerId("");
    setLightningCli("");
    setLightningDir("");
    setRpcFile("");
    setTestState("idle");
    setTestLog("");
  }, [backendSource, initial, open, preset, presetId, type]);

  const onPickType = (id: SyncBackendNetwork["id"]) => {
    setTypeId(id);
    setTestLog("");
    if (initial) {
      setPresetId("custom");
      return;
    }
    const nextType = SYNC_BACKEND_NETWORKS.find((candidate) => candidate.id === id);
    setBackendSource(nextType?.net === "LN" ? "custom" : "preset");
    setPresetId(nextType ? randomPreset(nextType)?.id ?? "custom" : "custom");
  };

  const testConnection = async () => {
    if (!effectiveUrl) return false;
    if (isCoreLightning) {
      setTestState("ok");
      setTestLog("Core Lightning read-only connection will be checked during wallet sync.");
      return true;
    }
    setTestState("testing");
    if (isElectrum) {
      try {
        const envelope = await testElectrum.mutateAsync({
          url: effectiveUrl,
          trust_self_signed: electrumUseSsl && trustSsl,
          certificate:
            electrumUseSsl && !trustSsl && certificate.trim()
              ? certificate.trim()
              : undefined,
          proxy:
            useProxy && proxyHost.trim() && proxyPort.trim()
              ? `${proxyHost.trim()}:${proxyPort.trim()}`
              : undefined,
        });
        const data = envelope.data;
        setTestState(data?.ok ? "ok" : "fail");
        setTestLog((data?.logs ?? []).join("\n"));
        return Boolean(data?.ok);
      } catch (error) {
        setTestState("fail");
        setTestLog(
          error instanceof Error ? error.message : "Electrum test failed.",
        );
        return false;
      }
    }
    try {
      const envelope = await testHttp.mutateAsync({
        url: effectiveUrl,
      });
      const data = envelope.data;
      setTestState(data?.ok ? "ok" : "fail");
      setTestLog((data?.logs ?? []).join("\n"));
      return Boolean(data?.ok);
    } catch (error) {
      setTestState("fail");
      setTestLog(error instanceof Error ? error.message : "HTTP test failed.");
      return false;
    }
  };

  // Editing an existing Core Lightning backend keeps redacted-but-set fields
  // from being treated as missing. The daemon never returns the rune itself;
  // it only signals presence via `auth === "apikey"` and the "Configured *"
  // sentinel strings used elsewhere in this file.
  const initialCoreLnHasRune =
    isCoreLightning && initial?.auth === "apikey";
  const initialCoreLnHasCommandoPeer =
    isCoreLightning &&
    initial?.commandoPeerId === CLN_PRESENCE_SENTINEL_COMMANDO_PEER;
  const initialCoreLnHasLightningDir =
    isCoreLightning &&
    initial?.lightningDir === CLN_PRESENCE_SENTINEL_LIGHTNING_DIR;
  const initialCoreLnHasRpcFile =
    isCoreLightning && initial?.rpcFile === CLN_PRESENCE_SENTINEL_RPC_FILE;

  const coreLightningModeValid = coreLightningBackendModeValid({
    commandoPeerId: commandoPeerId.trim(),
    rune: authVal.trim(),
    lightningDir: lightningDir.trim(),
    rpcFile: rpcFile.trim(),
    hadRune: initialCoreLnHasRune,
    hadCommandoPeerId: initialCoreLnHasCommandoPeer,
    hadLightningDir: initialCoreLnHasLightningDir,
    hadRpcFile: initialCoreLnHasRpcFile,
  });

  const canAdd =
    name.trim().length > 0 &&
    effectiveUrl.length > 0 &&
    (!isCoreLightning || coreLightningModeValid);
  const save = async () => {
    if (!canAdd) return;
    const normalizedUrl = effectiveUrl;
    let connected = testState === "ok" || isCoreLightning;
    setSaveState("saving");
    if (!connected) {
      connected = await testConnection();
      if (!connected) {
        setSaveState("idle");
        return;
      }
    }
    try {
      const authSecret = authVal.trim();
      const authPassword = authVal2.trim();
      await onSave({
        id: initial?.id ?? name.trim(),
        name: name.trim(),
        url: normalizedUrl,
        net: type.net,
        kind: selectedBackendKind,
        chain:
          type.net === "LIQUID"
            ? "liquid"
            : type.net === "LN"
              ? "bitcoin"
              : "bitcoin",
        network:
          type.net === "LIQUID"
            ? "liquidv1"
            : type.net === "LN"
              ? "main"
              : "main",
        health: initial ? "just checked - ok" : "just added - ok",
        on: connected,
        auth: isCoreLightning ? "apikey" : showAuth ? auth : "none",
        authHeader:
          showAuth && auth === "bearer" && authSecret
            ? `Bearer ${authSecret}`
            : undefined,
        token:
          (showAuth && auth === "apikey" && authSecret) ||
          (isCoreLightning && authSecret)
            ? authSecret
            : undefined,
        username:
          showAuth && auth === "basic" && authSecret ? authSecret : undefined,
        password:
          showAuth && auth === "basic" && authPassword
            ? authPassword
            : undefined,
        commandoPeerId:
          isCoreLightning && commandoPeerId.trim()
            ? commandoPeerId.trim()
            : undefined,
        lightningCli:
          isCoreLightning && lightningCli.trim() ? lightningCli.trim() : undefined,
        lightningDir:
          isCoreLightning && lightningDir.trim() ? lightningDir.trim() : undefined,
        rpcFile: isCoreLightning && rpcFile.trim() ? rpcFile.trim() : undefined,
        trustSsl:
          (showElectrumEndpointParts && electrumUseSsl) || isLnd
            ? trustSsl
            : undefined,
        infrastructureOwner: effectiveInfrastructureOwner,
        certificate:
          ((showElectrumEndpointParts && electrumUseSsl && !trustSsl) || isLnd) &&
          certificate.trim()
            ? certificate.trim()
            : undefined,
        proxy:
          showElectrumEndpointParts && useProxy && proxyHost.trim() && proxyPort.trim()
            ? { host: proxyHost.trim(), port: proxyPort.trim() }
            : null,
      });
    } catch (error) {
      setTestState("fail");
      setTestLog(error instanceof Error ? error.message : "Could not save backend.");
    } finally {
      setSaveState("idle");
    }
  };
  const isSavingBackend = saveState === "saving";

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="top-[6vh] max-h-[88vh] w-full max-w-[760px] translate-y-0 overflow-hidden p-0 sm:max-w-[760px]">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>{modalCopy.title}</DialogTitle>
          <DialogDescription>{modalCopy.description}</DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(88vh-150px)]">
          <div className="space-y-5 p-6">
            {showTypePicker ? (
              <section className="space-y-3">
                <div>
                  <Label>{modalCopy.selectorLabel}</Label>
                  <p className="text-sm text-muted-foreground">{type.desc}</p>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  {scopedTypes.map((backendType) => {
                    const active = backendType.id === typeId;
                    return (
                      <Button
                        key={backendType.id}
                        type="button"
                        variant="outline"
                        className={cn(
                          "h-auto min-h-[72px] items-center justify-start gap-3 whitespace-normal p-3 text-left",
                          selectorButtonClass(active),
                        )}
                        onClick={() => onPickType(backendType.id)}
                      >
                        <NetworkMark type={backendType} />
                        <span className="min-w-0 space-y-0.5">
                          <span className="block text-sm leading-tight font-medium">
                            {backendType.label}
                          </span>
                          <span className="block text-xs leading-tight text-muted-foreground">
                            {backendType.subtitle ?? backendType.net}
                          </span>
                        </span>
                      </Button>
                    );
                  })}
                </div>
              </section>
            ) : null}

            {isEditing ? (
              <section className="flex items-center gap-3 rounded-md border bg-muted/10 p-3">
                <NetworkMark type={type} />
                <div className="min-w-0">
                  <Label>Connection type</Label>
                  <p className="text-sm text-muted-foreground">
                    {type.label} · {preset?.label ?? selectedBackendKind}
                  </p>
                </div>
              </section>
            ) : null}

            {showSourcePicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Backend source</Label>
                  <p className="text-xs text-muted-foreground">
                    Use a third-party preset, or enter infrastructure you
                    operate.
                  </p>
                </div>
                <Tabs
                  value={backendSource}
                  onValueChange={(value) =>
                    setBackendSource(value as BackendSourceMode)
                  }
                >
                  <TabsList className="w-full justify-start sm:w-fit">
                    <TabsTrigger value="preset">
                      Third-party presets
                    </TabsTrigger>
                    <TabsTrigger value="custom">Custom backend</TabsTrigger>
                  </TabsList>
                </Tabs>
              </section>
            ) : null}

            {showPresetPicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Third-party endpoint</Label>
                  <p className="text-xs text-muted-foreground">
                    Pick from the bundled allowlist. Providers shown here have
                    a stated no-log policy.
                  </p>
                </div>
                <Select value={presetId} onValueChange={setPresetId}>
                  <SelectTrigger className="h-auto min-h-12 w-full py-2 text-left *:data-[slot=select-value]:line-clamp-none *:data-[slot=select-value]:justify-start">
                    <SelectValue placeholder="Choose a provider" />
                  </SelectTrigger>
                  <SelectContent>
                    {publicPresets.map((backendPreset) => {
                      const disabled = Boolean(backendPreset.disabled);
                      return (
                        <SelectItem
                          key={backendPreset.id}
                          value={backendPreset.id}
                          disabled={disabled}
                        >
                          <span className="flex min-w-0 items-center justify-start gap-2 text-left">
                            <PresetMark preset={backendPreset} net={type.net} />
                            <span className="min-w-0 space-y-0.5 text-left">
                              <span className="block truncate font-medium">
                                {presetDisplayName(backendPreset)}
                              </span>
                              <span className="block truncate text-xs text-muted-foreground">
                                {backendPreset.status ?? backendPreset.label}
                              </span>
                            </span>
                          </span>
                        </SelectItem>
                      );
                    })}
                  </SelectContent>
                </Select>
              </section>
            ) : null}

            {showCustomProtocolPicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Endpoint type</Label>
                  <p className="text-xs text-muted-foreground">
                    Choose the protocol your backend exposes.
                  </p>
                </div>
                <Tabs value={presetId} onValueChange={setPresetId}>
                  <TabsList className="w-full flex-wrap justify-start sm:w-fit">
                    {type.presets.map((backendPreset) => (
                      <TabsTrigger
                        key={backendPreset.id}
                        value={backendPreset.id}
                        disabled={backendPreset.disabled}
                      >
                        {backendPreset.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              </section>
            ) : null}

            <section className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="backend-name">Display name</Label>
                <Input
                  id="backend-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="My home node"
                />
                {isEditing ? (
                  <p className="text-xs text-muted-foreground">
                    Internal backend id: {initial?.id}
                  </p>
                ) : null}
              </div>
              {showElectrumEndpointParts ? (
                <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
                  <div className="space-y-2">
                    <Label htmlFor="backend-electrum-host">Host</Label>
                    <Input
                      id="backend-electrum-host"
                      value={electrumHost}
                      onChange={(event) => {
                        setElectrumHost(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder={
                        type.net === "LIQUID"
                          ? "liquid-electrum.example"
                          : "index.bitcoin-austria.at"
                      }
                      disabled={backendSource === "preset" && !isEditing}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="backend-electrum-port">Port</Label>
                    <Input
                      id="backend-electrum-port"
                      value={electrumPort}
                      onChange={(event) => {
                        setElectrumPort(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder={electrumUseSsl ? "50002" : "50001"}
                      disabled={backendSource === "preset" && !isEditing}
                    />
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="backend-url">Endpoint URL</Label>
                  <Input
                    id="backend-url"
                    value={url}
                    onChange={(event) => {
                      setUrl(event.target.value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="https://..."
                    disabled={backendSource === "preset" && !isEditing}
                  />
                </div>
              )}
            </section>

            {type.net !== "LN" && selectedKindIsExplorerApi ? (
              <div className="rounded-md border border-sky-500/25 bg-sky-500/5 p-3 text-xs text-muted-foreground">
                This Explorer API will also provide transaction links. Electrum
                and Fulcrum backends only provide wallet history sync.
              </div>
            ) : null}

            {type.net !== "LN" ? (
              <section className="space-y-3 rounded-md border p-3">
                <div
                  className={cn(
                    "flex items-start gap-2 rounded-md border p-3 text-xs",
                    connectionTrust.className,
                  )}
                >
                  <ConnectionTrustIcon
                    className="mt-0.5 size-4 shrink-0"
                    aria-hidden="true"
                  />
                  <div>
                    <div className="text-sm font-medium">
                      {connectionTrust.label}
                    </div>
                    <p className="mt-0.5 leading-relaxed">
                      {connectionTrust.note}
                    </p>
                  </div>
                </div>
                <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                  <div>
                    <Label>Infrastructure owner</Label>
                    <p className="text-xs text-muted-foreground">
                      This only changes privacy labeling; it does not change the endpoint.
                    </p>
                  </div>
                  <Tabs
                    value={infrastructureOwner}
                    onValueChange={(value) =>
                      setInfrastructureOwner(value as InfrastructureOwnership)
                    }
                  >
                    <TabsList className="w-full sm:w-fit">
                      <TabsTrigger value="self">Mine</TabsTrigger>
                      <TabsTrigger value="third_party">Third-party</TabsTrigger>
                    </TabsList>
                  </Tabs>
                </div>
              </section>
            ) : null}

            {isCoreLightning && (
              <section className="space-y-3">
                <div>
                  <Label>Core Lightning access</Label>
                  <p className="text-xs text-muted-foreground">
                    Use a restricted commando rune for least-privilege read-only sync,
                    or point at a local lightning-dir / rpc-file when running on the
                    same host.
                  </p>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="backend-commando-peer">Commando peer id</Label>
                    <Input
                      id="backend-commando-peer"
                      value={commandoPeerId}
                      onChange={(event) => {
                        setCommandoPeerId(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="02..."
                    />
                  </div>
                  <SecretField
                    id="backend-commando-rune"
                    label="Rune"
                    value={authVal}
                    onChange={(value) => {
                      setAuthVal(value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="readonly rune"
                  />
                  <div className="space-y-2">
                    <Label htmlFor="backend-lightning-cli">lightning-cli path</Label>
                    <Input
                      id="backend-lightning-cli"
                      value={lightningCli}
                      onChange={(event) => {
                        setLightningCli(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="lightning-cli"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="backend-lightning-dir">Lightning directory</Label>
                    <Input
                      id="backend-lightning-dir"
                      value={lightningDir}
                      onChange={(event) => {
                        setLightningDir(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="~/.lightning"
                    />
                  </div>
                  <div className="space-y-2 sm:col-span-2">
                    <Label htmlFor="backend-rpc-file">RPC file</Label>
                    <Input
                      id="backend-rpc-file"
                      value={rpcFile}
                      onChange={(event) => {
                        setRpcFile(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="lightning-rpc"
                    />
                    <p className="text-xs text-muted-foreground">
                      Local RPC file access is convenient but broader than a read-only rune.
                    </p>
                  </div>
                </div>
              </section>
            )}

            {showElectrumEndpointParts && backendSource === "custom" && (
              <details
                className="group rounded-md border bg-muted/10"
                open={trustSsl || Boolean(certificate) || useProxy || undefined}
              >
                <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-sm font-medium">
                  <span>Advanced connection settings</span>
                  <span className="text-xs text-muted-foreground">
                    TLS certificate and proxy options
                  </span>
                </summary>
                <section className="grid gap-3 border-t p-3 sm:grid-cols-2">
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm">
                    <span>
                      <span className="block font-medium">Use SSL</span>
                      <span className="text-muted-foreground">
                        Common Electrum SSL port is 50002.
                      </span>
                    </span>
                    <Switch
                      checked={electrumUseSsl}
                      onCheckedChange={(checked) => {
                        setElectrumUseSsl(checked);
                        if (!checked) {
                          setTrustSsl(false);
                          setCertificate("");
                        }
                        setElectrumPort((current) =>
                          current === "50002" || current === "50001"
                            ? checked
                              ? "50002"
                              : "50001"
                            : current,
                        );
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm">
                    <span>
                      <span className="block font-medium">
                        Trust self-signed certificate
                      </span>
                      <span className="text-muted-foreground">
                        For self-signed or private CA Electrum servers.
                      </span>
                    </span>
                    <Switch
                      checked={trustSsl}
                      disabled={!electrumUseSsl}
                      onCheckedChange={(checked) => {
                        setTrustSsl(checked);
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  <div className="space-y-2 sm:col-span-2">
                    <Label htmlFor="backend-certificate">Certificate</Label>
                    <Input
                      id="backend-certificate"
                      value={certificate}
                      onChange={(event) => {
                        setCertificate(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="Optional server certificate (.crt)"
                      disabled={!electrumUseSsl || trustSsl}
                    />
                    {electrumUseSsl && trustSsl ? (
                      <p className="text-xs text-muted-foreground">
                        Ignored while &ldquo;Trust self-signed certificate&rdquo;
                        is on.
                      </p>
                    ) : null}
                  </div>
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm sm:col-span-2">
                    <span>
                      <span className="block font-medium">Use proxy</span>
                      <span className="text-muted-foreground">
                        Optional Tor or SOCKS proxy for this endpoint.
                      </span>
                    </span>
                    <Switch
                      checked={useProxy}
                      onCheckedChange={(checked) => {
                        setUseProxy(checked);
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  {useProxy && (
                    <>
                      <div className="space-y-2">
                        <Label htmlFor="backend-proxy-host">Proxy host</Label>
                        <Input
                          id="backend-proxy-host"
                          value={proxyHost}
                          onChange={(event) => {
                            setProxyHost(event.target.value);
                            setTestState("idle");
                            setTestLog("");
                          }}
                          placeholder="127.0.0.1"
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="backend-proxy-port">Proxy port</Label>
                        <Input
                          id="backend-proxy-port"
                          value={proxyPort}
                          onChange={(event) => {
                            setProxyPort(event.target.value);
                            setTestState("idle");
                            setTestLog("");
                          }}
                          placeholder="9050"
                        />
                      </div>
                    </>
                  )}
                </section>
              </details>
            )}

            {isLnd && (
              <section className="grid gap-3 sm:grid-cols-2">
                <label className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm sm:col-span-2">
                  <span>
                    <span className="block font-medium">
                      Trust self-signed TLS
                    </span>
                    <span className="text-muted-foreground">
                      Use only for a local LND REST endpoint you control.
                    </span>
                  </span>
                  <Switch
                    checked={trustSsl}
                    onCheckedChange={(checked) => {
                      setTrustSsl(checked);
                      setTestState("idle");
                      setTestLog("");
                    }}
                  />
                </label>
                <div className="space-y-2 sm:col-span-2">
                  <Label htmlFor="backend-lnd-certificate">TLS certificate</Label>
                  <Input
                    id="backend-lnd-certificate"
                    value={certificate}
                    onChange={(event) => {
                      setCertificate(event.target.value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="Path to tls.cert or PEM contents"
                    disabled={trustSsl}
                  />
                </div>
              </section>
            )}

            {showAuth && (
              <section className="space-y-3">
                <Label>RPC authentication</Label>
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
                    label={isLnd ? "Read-only macaroon hex" : "API key"}
                    value={authVal}
                    onChange={setAuthVal}
                    placeholder={isLnd ? "0201036c6e64..." : "sk_live_..."}
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
            )}

            <div className="rounded-md border bg-muted/30 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    void testConnection();
                  }}
                  disabled={
                    !effectiveUrl || testState === "testing" || isSavingBackend
                  }
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
                  <span className="inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="size-4" />
                    Connected
                  </span>
                )}
                {testState === "fail" && (
                  <span className="inline-flex items-center gap-1 text-destructive">
                    <XCircle className="size-4" />
                    Could not reach endpoint
                  </span>
                )}
              </div>
              <textarea
                readOnly
                aria-label="Backend test connection log"
                value={testLog}
                className="mt-3 min-h-32 w-full resize-none rounded-md border bg-background p-3 font-mono text-xs leading-5"
              />
            </div>
          </div>
        </ScrollArea>

        <DialogFooter className="flex-row items-center justify-between gap-3 border-t px-6 py-4 sm:justify-between">
          <div>
            {initial && onDelete ? (
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                className="text-muted-foreground hover:text-destructive"
                aria-label={`Delete ${initial.name}`}
                title={`Delete ${initial.name}`}
                onClick={() => {
                  void onDelete(initial);
                }}
              >
                <Trash2 className="size-3.5" aria-hidden="true" />
              </Button>
            ) : null}
          </div>
          <div className="flex items-center gap-2">
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={!canAdd || isSavingBackend || testState === "testing"}
              onClick={() => {
                void save();
              }}
            >
              {isSavingBackend ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : null}
              {isSavingBackend
                ? "Connecting…"
                : testState === "ok"
                  ? isEditing
                    ? "Save backend"
                    : "Add sync backend"
                  : "Connect & save"}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
