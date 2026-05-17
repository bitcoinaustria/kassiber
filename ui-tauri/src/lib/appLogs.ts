export type AppLogLevel = "trace" | "debug" | "info" | "warning" | "error";

export type AppLogFieldType =
  | "text"
  | "boolean"
  | "number"
  | "duration_ms"
  | "address"
  | "xpub"
  | "xpriv"
  | "descriptor"
  | "txid"
  | "path"
  | "label"
  | "onion"
  | "api_key"
  | "email"
  | "ip"
  | "amount";

export interface AppLogField {
  type: AppLogFieldType;
  value: unknown;
}

export interface AppLogRecord {
  id: string;
  ts: string;
  level: AppLogLevel;
  module: string;
  file: string;
  line: number;
  msg: string;
  fields: Record<string, AppLogField>;
  spantrace?: AppLogRecord[];
}

export interface AppLogExportHeader {
  appVersion: string;
  os: string;
  timeRange: string;
  activeFilter: string;
  redaction: "redacted" | "raw" | "redacted-amounts";
  generatedAt?: string;
}

export interface AppLogRenderOptions {
  redacted: boolean;
  maskAmounts?: boolean;
}

export const APP_LOG_MAX_RECORDS = 5_000;
export const APP_LOG_MAX_BYTES = 2 * 1024 * 1024;

const LEVEL_WEIGHT: Record<AppLogLevel, number> = {
  trace: 10,
  debug: 20,
  info: 30,
  warning: 40,
  error: 50,
};

const SENSITIVE_FIELD_TYPES = new Set<AppLogFieldType>([
  "address",
  "xpub",
  "xpriv",
  "descriptor",
  "txid",
  "path",
  "label",
  "onion",
  "api_key",
  "email",
  "ip",
]);

let subscriptionLevel: AppLogLevel = "info";
let memoryRing: AppLogRecord[] = [];
let memoryRingBytes = 2;
const subscribers = new Set<() => void>();

const TEXT_BACKSTOP_PATTERNS: Array<[RegExp, string]> = [
  [
    /\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-private-key]",
  ],
  [
    /\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-extended-key]",
  ],
  [
    /\b(?:wpkh|sh|wsh|tr|pkh|combo)\([^)\n]{16,}\)/gi,
    "[redacted-wallet-material]",
  ],
  [/\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*/g, "Bearer [redacted]"],
  [
    /\b(api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)\b(\s*[:=]\s*)([^\s,;"']+)/gi,
    "$1$2[redacted]",
  ],
];

export function appLogLevels(): AppLogLevel[] {
  return ["trace", "debug", "info", "warning", "error"];
}

export function setAppLogSubscriptionLevel(level: AppLogLevel): void {
  subscriptionLevel = level;
}

export function getAppLogSubscriptionLevel(): AppLogLevel {
  return subscriptionLevel;
}

export function shouldEmitAppLog(level: AppLogLevel): boolean {
  return LEVEL_WEIGHT[level] >= LEVEL_WEIGHT[subscriptionLevel];
}

export function subscribeAppLogRecords(listener: () => void): () => void {
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
  };
}

export function getAppLogRecords(): AppLogRecord[] {
  return memoryRing;
}

export function getAppLogBufferSize(): number {
  return memoryRingBytes;
}

export function clearAppLogRecords(): void {
  memoryRing = [];
  memoryRingBytes = 2;
  notifySubscribers();
}

export function emitAppLog(
  record: Omit<AppLogRecord, "id" | "ts"> & {
    id?: string;
    ts?: string;
  },
): AppLogRecord | null {
  if (!shouldEmitAppLog(record.level)) return null;
  const next: AppLogRecord = {
    id: record.id ?? makeLogId(),
    ts: record.ts ?? new Date().toISOString(),
    level: record.level,
    module: record.module,
    file: record.file,
    line: record.line,
    msg: record.msg,
    fields: record.fields,
    spantrace: record.spantrace,
  };
  const ring = [...memoryRing, next];
  const bounded = enforceRingBounds(
    ring,
    memoryRingBytes + estimateRecordBytes(next),
  );
  memoryRing = bounded.records;
  memoryRingBytes = bounded.bytes;
  notifySubscribers();
  return next;
}

export function redactLogRecord(
  record: AppLogRecord,
  options: AppLogRenderOptions,
): AppLogRecord {
  if (!options.redacted) return record;
  return {
    ...record,
    msg: redactTextBackstop(record.msg),
    fields: redactFields(record.fields, options),
    spantrace: record.spantrace?.map((child) => redactLogRecord(child, options)),
  };
}

export function formatLogRecord(
  record: AppLogRecord,
  options: AppLogRenderOptions,
): string {
  const rendered = redactLogRecord(record, options);
  const fieldText = formatFields(rendered.fields);
  const location = `${rendered.module}:${rendered.file}:${rendered.line}`;
  return [
    formatTime(rendered.ts),
    rendered.level.toUpperCase().padEnd(7),
    location,
    rendered.msg,
    fieldText,
  ]
    .filter(Boolean)
    .join(" ");
}

export function exportLogRecords(
  records: AppLogRecord[],
  format: "md" | "log" | "jsonl",
  options: AppLogRenderOptions & { header: AppLogExportHeader },
): string {
  const rendered = records.map((record) => redactLogRecord(record, options));
  const rawWatermark = options.redacted ? "" : "RAW EXPORT - may contain wallet material";
  if (format === "jsonl") {
    const lines = [
      rawWatermark
        ? JSON.stringify({
            kind: "kassiber.log_export_watermark",
            ts: options.header.generatedAt ?? new Date().toISOString(),
            msg: rawWatermark,
          })
        : null,
      ...rendered.map((record) => JSON.stringify(record)),
    ].filter(Boolean);
    return `${lines.join("\n")}\n`;
  }
  const lines = rendered.map((record) => formatLogRecord(record, options));
  if (format === "log") {
    return rawWatermark ? `${rawWatermark}\n${lines.join("\n")}\n` : `${lines.join("\n")}\n`;
  }
  return [
    "# Kassiber log snapshot",
    "",
    rawWatermark ? `> ${rawWatermark}` : null,
    `- App version: ${options.header.appVersion}`,
    `- OS: ${options.header.os}`,
    `- Generated: ${options.header.generatedAt ?? new Date().toISOString()}`,
    `- Time range: ${options.header.timeRange}`,
    `- Active filter: ${options.header.activeFilter}`,
    `- Redaction: ${options.header.redaction}`,
    "",
    "```log",
    ...lines,
    "```",
    "",
  ]
    .filter((line) => line !== null)
    .join("\n");
}

export function logFilename(
  format: "md" | "log" | "jsonl",
  redaction: "redacted" | "raw" | "redacted-amounts",
  date = new Date(),
): string {
  const stamp = date.toISOString().slice(0, 16).replace(/:/g, "-");
  return `kassiber-${stamp}Z-${redaction}.${format}`;
}

export function stableMaskedValue(field: AppLogField): string {
  const value = String(field.value ?? "");
  if (!value) return "";
  switch (field.type) {
    case "address":
      return keepShort(value, 5, 4);
    case "xpub":
    case "xpriv":
      return keepShort(value, 6, 4);
    case "descriptor":
      return `wallet#${stableHash(value)}`;
    case "txid":
      return `txid#${stableHash(value)}`;
    case "path":
      return maskPath(value);
    case "label":
      return `wallet#${stableHash(value)}`;
    case "onion":
      return `onion#${stableHash(value)}`;
    case "api_key":
      return `key#${stableHash(value)}`;
    case "email":
      return `email#${stableHash(value)}`;
    case "ip":
      return `ip#${stableHash(value)}`;
    case "amount":
      return `amount#${stableHash(value)}`;
    default:
      return value;
  }
}

function redactField(
  field: AppLogField,
  options: AppLogRenderOptions,
): AppLogField {
  if (field.type === "amount" && !options.maskAmounts) return field;
  if (field.type === "text" && typeof field.value === "string") {
    return { ...field, value: redactTextBackstop(field.value) };
  }
  if (!SENSITIVE_FIELD_TYPES.has(field.type) && field.type !== "amount") {
    return field;
  }
  return { type: "text", value: stableMaskedValue(field) };
}

function redactFields(
  fields: Record<string, AppLogField>,
  options: AppLogRenderOptions,
): Record<string, AppLogField> {
  const used = new Map<string, number>();
  const entries = Object.entries(fields).map(([name, field]) => {
    const baseName = redactedFieldName(name, field);
    const seen = used.get(baseName) ?? 0;
    used.set(baseName, seen + 1);
    const renderedName = seen === 0 ? baseName : `${baseName}_${seen + 1}`;
    return [renderedName, redactField(field, options)] as const;
  });
  return Object.fromEntries(entries);
}

function redactedFieldName(name: string, field: AppLogField): string {
  if (field.type === "amount") return name;
  if (!SENSITIVE_FIELD_TYPES.has(field.type)) return name;
  if (
    field.type === "xpub" ||
    field.type === "xpriv" ||
    field.type === "descriptor"
  ) {
    return "wallet_material";
  }
  return field.type;
}

function redactTextBackstop(value: string): string {
  return TEXT_BACKSTOP_PATTERNS.reduce(
    (current, [pattern, replacement]) => current.replace(pattern, replacement),
    value,
  );
}

function formatFields(fields: Record<string, AppLogField>): string {
  const parts = Object.entries(fields).map(([name, field]) => {
    const value =
      typeof field.value === "string" ||
      typeof field.value === "number" ||
      typeof field.value === "boolean"
        ? String(field.value)
        : JSON.stringify(field.value);
    return `${name}=${value}`;
  });
  return parts.length ? parts.join(" ") : "";
}

interface RingSnapshot {
  records: AppLogRecord[];
  bytes: number;
}

function enforceRingBounds(
  records: AppLogRecord[],
  estimatedBytes: number,
): RingSnapshot {
  let bytes = estimatedBytes;
  const firstIndex = Math.max(0, records.length - APP_LOG_MAX_RECORDS);
  for (let index = 0; index < firstIndex; index += 1) {
    bytes -= estimateRecordBytes(records[index]);
  }
  let next = firstIndex > 0 ? records.slice(firstIndex) : records;
  while (next.length > 0 && bytes > APP_LOG_MAX_BYTES) {
    const dropCount = Math.max(1, Math.ceil(next.length * 0.1));
    for (let index = 0; index < dropCount; index += 1) {
      bytes -= estimateRecordBytes(next[index]);
    }
    next = next.slice(dropCount);
  }
  return { records: next, bytes: Math.max(2, bytes) };
}

function estimateRecordBytes(record: AppLogRecord): number {
  return JSON.stringify(record).length + 1;
}

function notifySubscribers(): void {
  for (const subscriber of subscribers) subscriber();
}

function makeLogId(): string {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `log-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function formatTime(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toISOString();
}

function keepShort(value: string, head: number, tail: number): string {
  if (value.length <= head + tail + 1) return value;
  return `${value.slice(0, head)}...${value.slice(-tail)}`;
}

function maskPath(value: string): string {
  const normalized = value.replace(/\\/g, "/");
  const name = normalized.split("/").filter(Boolean).pop() ?? "path";
  return `~/.../${name}`;
}

function stableHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0").slice(0, 4);
}
