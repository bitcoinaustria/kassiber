/**
 * Shared constants/labels for Lightning surfaces.
 *
 * Mirrors `kassiber/core/lightning/profitability.py::DEFAULT_OPEN_COST_SAT`
 * and the wallet kinds tuple. When the Python default changes, update this
 * file in the same commit so the mock daemon and Reports panel agree with
 * what live adapters return.
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
 * Inputs to the Core Lightning backend save validation. Everything is
 * either a user-typed string or a snapshot of what the existing backend
 * already had configured (presence-only — the daemon never sends the rune
 * back, so editing keeps the previous value untouched when this form
 * leaves the field empty).
 */
export interface CoreLightningBackendFormState {
  /** Trimmed value of the commando peer id input. */
  commandoPeerId: string;
  /** Trimmed value of the rune (auth) input. Empty during edit means "keep current". */
  rune: string;
  /** Trimmed value of the local lightning-dir input. */
  lightningDir: string;
  /** Trimmed value of the local rpc-file input. */
  rpcFile: string;
  /** True when editing an existing backend that already had a rune stored. */
  hadRune: boolean;
  /** True when editing an existing backend that already had a commando peer id stored. */
  hadCommandoPeerId: boolean;
  /** True when editing an existing backend that already had a lightning_dir stored. */
  hadLightningDir: boolean;
  /** True when editing an existing backend that already had an rpc_file stored. */
  hadRpcFile: boolean;
}

/**
 * Validate the Core Lightning section of the Add/Edit Backend modal.
 *
 * Two transport modes are accepted:
 *
 * - **Commando rune** (recommended): requires both a commando peer id and
 *   a rune token. During create the user must type both; during edit a
 *   blank field falls back to the value already stored on the backend.
 * - **Local RPC**: requires either a lightning_dir or an rpc_file path so
 *   the adapter can locate the unix socket without commando credentials.
 *
 * The previous implementation forced commando credentials on every save
 * (`canAdd && (commandoPeerId && rune)`), which blocked editing an
 * existing working backend (the daemon redacts the rune back to empty
 * after save) and refused the documented local rpc_file path. (M-2.)
 */
export function coreLightningBackendModeValid(
  state: CoreLightningBackendFormState,
): boolean {
  const peerIdProvided = Boolean(state.commandoPeerId) || state.hadCommandoPeerId;
  const runeProvided = Boolean(state.rune) || state.hadRune;
  const commandoValid = peerIdProvided && runeProvided;

  const localDirProvided = Boolean(state.lightningDir) || state.hadLightningDir;
  const localRpcProvided = Boolean(state.rpcFile) || state.hadRpcFile;
  const localValid = localDirProvided || localRpcProvided;

  return commandoValid || localValid;
}
