import { joinQRs } from "bbqr";

import { detectWalletMaterial } from "./walletMaterialFormat";

const BBQR_PREFIX = "B$";
const BBQR_HEADER_LENGTH = 8;

export type QrScanMode = "auto" | "single" | "bbqr";

export interface BbqrCollectorState {
  frames: Record<number, string>;
  total?: number;
  encoding?: string;
  fileType?: string;
}

export interface BbqrProgress {
  received: number;
  total: number;
  fileType: string;
}

export type WalletMaterialScanResult =
  | { status: "single"; material: string }
  | {
      status: "bbqr_progress";
      state: BbqrCollectorState;
      progress: BbqrProgress;
    }
  | {
      status: "bbqr_complete";
      state: BbqrCollectorState;
      progress: BbqrProgress;
      material: string;
      fileType: string;
    }
  | { status: "ignored"; message: string }
  | { status: "error"; message: string };

interface ParsedBbqrFrame {
  total: number;
  index: number;
  encoding: string;
  fileType: string;
}

function parseBase36Pair(value: string): number | null {
  if (!/^[0-9A-Z]{2}$/.test(value)) return null;
  const parsed = Number.parseInt(value, 36);
  return Number.isFinite(parsed) ? parsed : null;
}

export function isBbqrFrame(value: string): boolean {
  return value.trim().toUpperCase().startsWith(BBQR_PREFIX);
}

function parseBbqrFrame(value: string): ParsedBbqrFrame | null {
  const trimmed = value.trim().toUpperCase();
  if (!trimmed.startsWith(BBQR_PREFIX) || trimmed.length < BBQR_HEADER_LENGTH) {
    return null;
  }
  const total = parseBase36Pair(trimmed.slice(4, 6));
  const index = parseBase36Pair(trimmed.slice(6, 8));
  if (total === null || index === null || total < 1 || index >= total) {
    return null;
  }
  return {
    encoding: trimmed[2] ?? "",
    fileType: trimmed[3] ?? "",
    total,
    index,
  };
}

function progressFor(state: BbqrCollectorState): BbqrProgress {
  const received = Object.keys(state.frames).length;
  return {
    received,
    total: state.total ?? received,
    fileType: state.fileType ?? "?",
  };
}

function decodeBbqrMaterial(parts: string[]): { material: string; fileType: string } {
  const joined = joinQRs(parts);
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const material = decoder.decode(joined.raw).trim();
  if (!material) {
    throw new Error("The scanned BBQR payload was empty.");
  }
  if (joined.fileType === "U" || joined.fileType === "J") {
    return { material, fileType: joined.fileType };
  }
  if (joined.fileType === "B") {
    const detection = detectWalletMaterial(material);
    if (detection.kind !== "unknown" && detection.kind !== "empty") {
      return { material, fileType: joined.fileType };
    }
  }
  throw new Error(
    `BBQR type ${joined.fileType} is not a wallet export or descriptor.`,
  );
}

export function processWalletMaterialQrScan(
  value: string,
  mode: QrScanMode,
  currentState: BbqrCollectorState,
): WalletMaterialScanResult {
  const scanned = value.trim();
  if (!scanned) {
    return { status: "ignored", message: "Empty QR frame ignored." };
  }
  const parsed = parseBbqrFrame(scanned);
  if (!parsed) {
    if (mode === "bbqr") {
      return {
        status: "ignored",
        message: "Waiting for a BBQR frame.",
      };
    }
    return { status: "single", material: scanned };
  }
  if (mode === "single") {
    return {
      status: "ignored",
      message: "BBQR frame seen; switch to Auto or BBQR to collect it.",
    };
  }

  const mismatch =
    (currentState.total !== undefined && currentState.total !== parsed.total) ||
    (currentState.encoding !== undefined &&
      currentState.encoding !== parsed.encoding) ||
    (currentState.fileType !== undefined &&
      currentState.fileType !== parsed.fileType);
  const baseState: BbqrCollectorState = mismatch
    ? { frames: {} }
    : currentState;
  const nextState: BbqrCollectorState = {
    frames: { ...baseState.frames, [parsed.index]: scanned },
    total: parsed.total,
    encoding: parsed.encoding,
    fileType: parsed.fileType,
  };
  const progress = progressFor(nextState);
  if (progress.received < parsed.total) {
    return { status: "bbqr_progress", state: nextState, progress };
  }
  try {
    const parts = Object.values(nextState.frames);
    const decoded = decodeBbqrMaterial(parts);
    return {
      status: "bbqr_complete",
      state: nextState,
      progress,
      material: decoded.material,
      fileType: decoded.fileType,
    };
  } catch (error) {
    return {
      status: "error",
      message:
        error instanceof Error ? error.message : "Could not decode BBQR frames.",
    };
  }
}

export function emptyBbqrCollectorState(): BbqrCollectorState {
  return { frames: {} };
}
