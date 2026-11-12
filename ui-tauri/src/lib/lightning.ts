/**
 * Shared constants/labels for Lightning surfaces.
 *
 * Mirrors `kassiber/core/lightning/profitability.py::DEFAULT_OPEN_COST_SAT`
 * and the wallet kinds tuple. When the Python default changes, update this
 * file in the same commit so the Reports panel agrees with live adapters.
 */

export const DEFAULT_OPEN_COST_SAT = 2_500;

// Wallet kinds the Python daemon treats as Lightning nodes. The UI
// catalog uses `core-ln` (hyphen) for display while the wallets table
// stores `coreln`; both spellings are accepted across the boundary.
export const LIGHTNING_CONNECTION_KINDS: ReadonlySet<string> = new Set([
  "core-ln",
  "coreln",
  "lnd",
  "nwc",
]);

export interface LightningCapabilities {
  nodeSnapshot: boolean;
  routingProfitability: boolean;
  channelBalances: boolean;
  channelLifecycle: boolean;
  forwardEvents: boolean;
  invoiceActivity: boolean;
  paymentActivity: boolean;
  onchainBalance: boolean;
}

export type LightningCapabilityKey = keyof LightningCapabilities;

export const EMPTY_LIGHTNING_CAPABILITIES: LightningCapabilities = {
  nodeSnapshot: false,
  routingProfitability: false,
  channelBalances: false,
  channelLifecycle: false,
  forwardEvents: false,
  invoiceActivity: false,
  paymentActivity: false,
  onchainBalance: false,
};

const LEGACY_NODE_SNAPSHOT_KINDS = new Set(["core-ln", "coreln", "lnd"]);

function normalizeLightningCapabilities(value: unknown) {
  if (!value || typeof value !== "object") return null;
  const raw = value as Partial<Record<LightningCapabilityKey, unknown>>;
  return {
    nodeSnapshot: raw.nodeSnapshot === true,
    routingProfitability: raw.routingProfitability === true,
    channelBalances: raw.channelBalances === true,
    channelLifecycle: raw.channelLifecycle === true,
    forwardEvents: raw.forwardEvents === true,
    invoiceActivity: raw.invoiceActivity === true,
    paymentActivity: raw.paymentActivity === true,
    onchainBalance: raw.onchainBalance === true,
  } satisfies LightningCapabilities;
}

export function lightningCapabilitiesForConnection(connection: {
  kind?: string | null;
  lightningCapabilities?: unknown;
}): LightningCapabilities {
  const declared = normalizeLightningCapabilities(
    connection.lightningCapabilities,
  );
  if (declared) return declared;
  const kind = connection.kind ?? "";
  if (!LEGACY_NODE_SNAPSHOT_KINDS.has(kind)) return EMPTY_LIGHTNING_CAPABILITIES;
  return {
    ...EMPTY_LIGHTNING_CAPABILITIES,
    nodeSnapshot: true,
    routingProfitability: true,
  };
}

export function connectionSupportsLightningCapability(
  connection: {
    kind?: string | null;
    lightningCapabilities?: unknown;
  },
  capability: LightningCapabilityKey,
): boolean {
  return lightningCapabilitiesForConnection(connection)[capability];
}

// Sentinel strings the daemon emits when reporting that a Core Lightning
// backend has secret-bearing fields configured but redacted. The settings
// form replaces these with empty inputs so users do not see "Configured
// peer" inside the edit field.
export const CLN_PRESENCE_SENTINEL_COMMANDO_PEER = "Configured peer";
export const CLN_PRESENCE_SENTINEL_LIGHTNING_DIR = "Configured directory";
export const CLN_PRESENCE_SENTINEL_RPC_FILE = "Configured RPC file";

/**
 * Whether a Core Lightning backend is configured for commando: a
 * `cln://commando` URL, a stored rune (`auth === "apikey"`), or a commando
 * peer id. The core refuses these before starting `lightning-cli`, because
 * `lightning-cli` accepts a rune only on its command line, where other local
 * users can read it (same test as `_refuse_commando` in
 * `kassiber/core/lightning/cln.py`).
 */
export function isCoreLightningCommandoBackend(backend: {
  kind?: string | null;
  url?: string | null;
  auth?: string | null;
  commandoPeerId?: string | null;
}): boolean {
  if ((backend.kind ?? "").toLowerCase() !== "coreln") return false;
  return (
    (backend.url ?? "").toLowerCase().startsWith("cln://commando") ||
    backend.auth === "apikey" ||
    Boolean(backend.commandoPeerId?.trim())
  );
}

/**
 * Core Lightning connects only through local RPC. A blank URL or a commando
 * URL (which the core refuses) becomes the local one.
 */
export function coreLightningUrl(raw: string): string {
  const trimmed = raw.trim();
  return !trimmed || trimmed.toLowerCase().startsWith("cln://commando")
    ? "cln://local"
    : trimmed;
}

/**
 * Inputs to the Core Lightning backend save validation. Everything is
 * either a user-typed string or a snapshot of what the existing backend
 * already had configured (presence-only: the daemon reports lightning_dir
 * and rpc_file as "Configured *" sentinels, so editing keeps the previous
 * value untouched when this form leaves the field empty).
 */
export interface CoreLightningBackendFormState {
  /** Trimmed value of the lightning-cli path or wrapper input. */
  lightningCli: string;
  /** Trimmed value of the local lightning-dir input. */
  lightningDir: string;
  /** Trimmed value of the local rpc-file input. */
  rpcFile: string;
  /** True when editing an existing backend that already had a lightning_dir stored. */
  hadLightningDir: boolean;
  /** True when editing an existing backend that already had an rpc_file stored. */
  hadRpcFile: boolean;
}

/**
 * Validate the Core Lightning section of the Add/Edit Backend modal.
 *
 * Only local access is supported: `lightning-cli` (or a wrapper the user
 * controls) reaching the node's RPC socket, located through a lightning_dir,
 * an rpc_file, or the wrapper itself. Commando is refused by the core because
 * `lightning-cli` takes the rune only on its command line, so the form offers
 * no rune or peer fields.
 */
export function coreLightningBackendModeValid(
  state: CoreLightningBackendFormState,
): boolean {
  return (
    Boolean(state.lightningCli) ||
    Boolean(state.lightningDir) ||
    state.hadLightningDir ||
    Boolean(state.rpcFile) ||
    state.hadRpcFile
  );
}
