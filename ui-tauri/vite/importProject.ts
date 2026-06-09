import {
  closeSync,
  existsSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
} from "node:fs";
import path from "node:path";

const DEFAULT_STATE_DIR = ".kassiber";
const DEFAULT_DATA_DIR = "data";
const DB_FILENAMES = ["kassiber.sqlite3", "satbooks.sqlite3"] as const;
const SQLITE_HEADER = Buffer.from("SQLite format 3\0", "ascii");

export interface BridgeImportProjectSelection {
  stateRoot: string;
  dataRoot: string;
  database: string;
  encrypted: boolean;
}

function expandUserPath(rawPath: string) {
  if (rawPath === "~" || rawPath.startsWith("~/")) {
    const home = process.env.HOME;
    if (home) {
      return rawPath === "~" ? home : path.join(home, rawPath.slice(2));
    }
  }
  return rawPath;
}

function isMissingFileError(error: unknown) {
  return (
    error instanceof Error &&
    "code" in error &&
    (error as { code?: unknown }).code === "ENOENT"
  );
}

function inspectDataRootCandidate(candidate: string): string | null {
  let metadata;
  try {
    metadata = lstatSync(candidate);
  } catch (error) {
    if (isMissingFileError(error)) return null;
    throw new Error(
      `Kassiber data folder candidate could not be inspected: ${String(error)}`,
    );
  }
  if (metadata.isSymbolicLink()) {
    throw new Error("Kassiber data folders must not be symlinks.");
  }
  if (!metadata.isDirectory()) {
    return null;
  }
  try {
    return realpathSync(candidate);
  } catch (error) {
    throw new Error(`Kassiber data folder could not be opened: ${String(error)}`);
  }
}

function readFilePrefix(filePath: string, byteCount: number) {
  const fd = openSync(filePath, "r");
  try {
    const buffer = Buffer.alloc(byteCount);
    const read = readSync(fd, buffer, 0, byteCount, 0);
    return buffer.subarray(0, read);
  } finally {
    closeSync(fd);
  }
}

function databaseIsEncrypted(database: string) {
  const header = readFilePrefix(database, SQLITE_HEADER.length);
  if (header.length === 0) return false;
  if (header.length < SQLITE_HEADER.length) return true;
  return !header.equals(SQLITE_HEADER);
}

function plaintextDatabaseLooksLikeKassiber(database: string) {
  const prefix = readFilePrefix(database, 1024 * 1024)
    .toString("latin1")
    .toLowerCase();
  return [
    "create table",
    "settings",
    "workspaces",
    "profiles",
    "workspace_id",
    "fiat_currency",
  ].every((needle) => prefix.includes(needle));
}

function inspectDatabaseCandidate(database: string): boolean | null {
  let metadata;
  try {
    metadata = lstatSync(database);
  } catch (error) {
    if (isMissingFileError(error)) return null;
    throw new Error(
      `Kassiber database candidate could not be inspected: ${String(error)}`,
    );
  }
  if (metadata.isSymbolicLink()) {
    throw new Error("Kassiber database files must not be symlinks.");
  }
  if (!metadata.isFile()) {
    return null;
  }
  if (metadata.size === 0) {
    throw new Error("Kassiber database file is empty.");
  }

  const encrypted = databaseIsEncrypted(database);
  if (!encrypted && !plaintextDatabaseLooksLikeKassiber(database)) {
    throw new Error(
      "Selected SQLite file does not contain Kassiber workspace/profile tables.",
    );
  }
  return encrypted;
}

function isManagedStateRoot(candidate: string) {
  return (
    path.basename(candidate) === DEFAULT_STATE_DIR ||
    existsSync(path.join(candidate, "config", "settings.json"))
  );
}

function resolveImportDataRoot(
  candidate: string,
): { dataRoot: string; database: string; encrypted: boolean } | null {
  let direct: { dataRoot: string; database: string; encrypted: boolean } | null =
    null;
  for (const filename of DB_FILENAMES) {
    const database = path.join(candidate, filename);
    const encrypted = inspectDatabaseCandidate(database);
    if (encrypted !== null) {
      direct = { dataRoot: candidate, database, encrypted };
      break;
    }
  }

  let nested: { dataRoot: string; database: string; encrypted: boolean } | null =
    null;
  const nestedCandidate = path.join(candidate, DEFAULT_DATA_DIR);
  const nestedDataRoot = inspectDataRootCandidate(nestedCandidate);
  if (nestedDataRoot) {
    for (const filename of DB_FILENAMES) {
      const database = path.join(nestedDataRoot, filename);
      const encrypted = inspectDatabaseCandidate(database);
      if (encrypted !== null) {
        nested = { dataRoot: nestedDataRoot, database, encrypted };
        break;
      }
    }
  }

  if (direct && nested) {
    if (isManagedStateRoot(candidate)) return nested;
    throw new Error(
      "Selected folder contains Kassiber databases both directly and under data/. Choose the exact data folder to import.",
    );
  }
  return direct ?? nested;
}

export function inspectImportProjectDirectory(
  selectedPath: string,
): BridgeImportProjectSelection {
  const expanded = expandUserPath(selectedPath.trim());
  const canonical = inspectDataRootCandidate(expanded);
  if (!canonical) {
    throw new Error(
      "Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself.",
    );
  }
  const resolved = resolveImportDataRoot(canonical);
  if (!resolved) {
    throw new Error(
      "Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself.",
    );
  }
  const stateRoot =
    path.basename(resolved.dataRoot) === DEFAULT_DATA_DIR
      ? path.dirname(resolved.dataRoot)
      : resolved.dataRoot;
  return {
    stateRoot,
    dataRoot: resolved.dataRoot,
    database: resolved.database,
    encrypted: resolved.encrypted,
  };
}
