export type AppLogLevel = "trace" | "debug" | "info" | "warning" | "error";

export type AppLogFieldType =
  | "text"
  | "boolean"
  | "number"
  | "duration_ms"
  | "address"
  | "url"
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

export type AppLogRedactionMode = "high_signal" | "public_safe";

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
  redaction:
    | "redacted"
    | "raw"
    | "redacted-amounts"
    | AppLogRedactionMode;
  generatedAt?: string;
}

export interface AppLogRenderOptions {
  redacted: boolean;
  maskAmounts?: boolean;
  mode?: AppLogRedactionMode;
}

export interface AppSupportBundleOptions {
  issueDescription: string;
  header: AppLogExportHeader;
  maxEvents?: number;
  contextRadius?: number;
  includeAiProvenance?: boolean;
  mode?: AppLogRedactionMode;
}

export const APP_LOG_MAX_RECORDS = 10_000;
export const APP_LOG_MAX_BYTES = 4 * 1024 * 1024;

const SECRET_FLOOR_FIELD_TYPES = new Set<AppLogFieldType>([
  "api_key",
  "descriptor",
  "xpriv",
  "xpub",
]);

const OPERATIONAL_FIELD_TYPES = new Set<AppLogFieldType>([
  "address",
  "email",
  "ip",
  "label",
  "onion",
  "path",
  "txid",
  "url",
]);

let memoryRing: AppLogRecord[] = [];
let memoryRingBytes = 2;
const subscribers = new Set<() => void>();

type TextBackstopReplacement = string | ((...matches: string[]) => string);

const BARE_BIP39_WORD_RUN_PATTERNS: Array<[RegExp, TextBackstopReplacement]> =
  [24, 21, 18, 15, 12].map((wordCount) => [
    new RegExp(
      `\\b(?:[a-z]{3,8}\\s+){${wordCount - 1}}[a-z]{3,8}\\b`,
      "g",
    ),
    "[redacted-seed-phrase]",
  ]);

const SECRET_FLOOR_TEXT_PATTERNS: Array<[RegExp, TextBackstopReplacement]> = [
  [
    /\b(mnemonic|recovery[_-]?phrase|seed(?:[_-]?phrase)?)\b(\s*[:=]\s*)(.+)$/gim,
    "$1$2[redacted]",
  ],
  [
    /\b((?:https?|tcp|ssl):\/\/)([^/\s:@]+):([^@\s/]+)@/gi,
    "$1[redacted-credentials]@",
  ],
  ...BARE_BIP39_WORD_RUN_PATTERNS,
  [
    /\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-private-key]",
  ],
  [
    /\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b/g,
    "[redacted-extended-key]",
  ],
  [
    /\b(wpkh|sh|wsh|tr|pkh|combo)\([^\n]{16,400}\)(?:#[a-z0-9]{8})?/gi,
    (match) => maskDescriptor(match),
  ],
  [/\b[Bb]earer\s+[A-Za-z0-9._~+/-]+=*/g, "Bearer [redacted]"],
  [
    /\b(api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)\b(\s*[:=]\s*)([^\s,;"']+)/gi,
    "$1$2[redacted]",
  ],
  // JSON-shaped assignments, e.g. a logged object `{"api_key":"sk-..."}`. The
  // pattern above stops at the key's closing quote, so the quoted value
  // survives without this; keep the key and redact the value in place.
  [
    /("(?:api[_-]?key|auth[_-]?header|cookie|descriptor|mnemonic|recovery[_-]?phrase|passphrase|password|secret|seed(?:[_-]?(?:phrase|words))?|token|xprv)"\s*:\s*)"[^"]*"/gi,
    '$1"[redacted]"',
  ],
];

const PUBLIC_SAFE_AMOUNT_NUMBER =
  "[+-]?(?:(?:\\d{1,3}(?:[,_ .]\\d{3})+)|\\d+)(?:[.,]\\d+)?";
const PUBLIC_SAFE_AMOUNT_UNITS =
  "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF";
const PUBLIC_SAFE_PAIR_UNITS = PUBLIC_SAFE_AMOUNT_UNITS;
const PUBLIC_SAFE_CURRENCY_SYMBOLS = "\\u20ac$\\u00a3\\u00a5\\u20bf";

const PUBLIC_SAFE_TEXT_PATTERNS: Array<[RegExp, TextBackstopReplacement]> = [
  [
    /\b(?:https?|tcp|ssl):\/\/[^\s,;"')\]}]+/gi,
    "[redacted-url]",
  ],
  [
    /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi,
    "[redacted-email]",
  ],
  [
    new RegExp(
      `\\b(?:${PUBLIC_SAFE_PAIR_UNITS})[/-](?:${PUBLIC_SAFE_PAIR_UNITS})\\s*(?::|=|at|rate)?\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`,
      "gi",
    ),
    "[redacted-rate]",
  ],
  [
    new RegExp(
      `\\b(?:${PUBLIC_SAFE_AMOUNT_UNITS})\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`,
      "gi",
    ),
    "[redacted-amount]",
  ],
  [
    new RegExp(
      `\\b${PUBLIC_SAFE_AMOUNT_NUMBER}\\s*(?:${PUBLIC_SAFE_AMOUNT_UNITS})\\b`,
      "gi",
    ),
    "[redacted-amount]",
  ],
  [
    new RegExp(`[${PUBLIC_SAFE_CURRENCY_SYMBOLS}]\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`, "g"),
    "[redacted-amount]",
  ],
  [
    new RegExp(`\\b${PUBLIC_SAFE_AMOUNT_NUMBER}\\s*[${PUBLIC_SAFE_CURRENCY_SYMBOLS}]`, "g"),
    "[redacted-amount]",
  ],
  [
    /\b(?:bc1|tb1|bcrt1|lq1|ex1)[023456789acdefghjklmnpqrstuvwxyz]{20,90}\b/gi,
    "[redacted-address]",
  ],
  [
    /\b[13][1-9A-HJ-NP-Za-km-z]{25,34}\b/g,
    "[redacted-address]",
  ],
  [
    /\b[0-9a-f]{64}\b/gi,
    "[redacted-txid]",
  ],
  [
    /\b[A-Za-z0-9.-]{16,}\.onion\b/gi,
    "[redacted-onion]",
  ],
  [
    /(?:^|[\s"'(])(?:\/Users|\/home|\/var|\/private|\/tmp)\/[^\s,;"')\]}]+/g,
    (match) => `${match[0] === "/" ? "" : match[0]}[redacted-path]`,
  ],
];

export function appLogLevels(): AppLogLevel[] {
  return ["trace", "debug", "info", "warning", "error"];
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
): AppLogRecord {
  const next: AppLogRecord = {
    id: record.id ?? makeLogId(),
    ts: record.ts ?? new Date().toISOString(),
    level: record.level,
    module: record.module,
    file: record.file,
    line: record.line,
    msg: redactSecretFloorText(record.msg),
    fields: secretFloorFieldsAtInsert(record.fields),
    spantrace: record.spantrace?.map(secretFloorRecordAtInsert),
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
    msg: redactTextForMode(record.msg, options.mode ?? "public_safe"),
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

export function supportBundleFilename(date = new Date()): string {
  const stamp = date.toISOString().slice(0, 16).replace(/:/g, "-");
  return `kassiber-support-${stamp}Z.support.jsonl`;
}

export function exportSupportBundleRecords(
  records: AppLogRecord[],
  options: AppSupportBundleOptions,
): string {
  const generatedAt = options.header.generatedAt ?? new Date().toISOString();
  const mode = options.mode ?? "high_signal";
  const maxEvents = options.maxEvents ?? 1000;
  const contextRadius = options.contextRadius ?? 12;
  const includeAiProvenance = options.includeAiProvenance ?? true;
  const exportedRecords = records.slice(-maxEvents);
  const renderOptions: AppLogRenderOptions = {
    redacted: true,
    maskAmounts: mode === "public_safe",
    mode,
  };
  const events = exportedRecords.map((record) =>
    redactLogRecord(record, renderOptions),
  );
  const failureIndexes = indexesForFailures(events);
  const failureContextIndexes = contextIndexesForFailures(
    events,
    failureIndexes,
    contextRadius,
  );
  const aiIndexes = includeAiProvenance
    ? events
        .map((record, index) => (isAiProvenanceRecord(record) ? index : -1))
        .filter((index) => index >= 0)
    : [];
  const omittedEvents = Math.max(0, records.length - exportedRecords.length);

  const lines = [
    {
      kind: "kassiber.support_bundle.manifest",
      schema_version: 1,
      generated_at: generatedAt,
      app_version: options.header.appVersion,
      os: options.header.os,
      time_range: options.header.timeRange,
      active_filter: options.header.activeFilter,
      redaction: mode,
      public_safe: mode === "public_safe",
      format_note: supportBundleFormatNote(mode),
      sections: {
        issue: 1,
        redaction_report: 1,
        events: events.length,
        last_failures: failureContextIndexes.size,
        ai_provenance: aiIndexes.length,
        diagnostics: 1,
      },
    },
    {
      kind: "kassiber.support_bundle.issue",
      schema_version: 1,
      description: redactTextForMode(options.issueDescription.trim(), mode),
    },
    {
      kind: "kassiber.support_bundle.redaction_report",
      schema_version: 1,
      ...redactionReportForRecords(exportedRecords, omittedEvents, mode),
    },
    {
      kind: "kassiber.support_bundle.diagnostics",
      schema_version: 1,
      summary: {
        events_included: events.length,
        events_omitted_from_start: omittedEvents,
        failures_detected: failureIndexes.length,
        ai_provenance_records: aiIndexes.length,
        buffer_time_range: options.header.timeRange,
      },
    },
    ...events.map((record, index) => ({
      kind: "kassiber.support_bundle.event",
      schema_version: 1,
      index,
      record,
    })),
    ...Array.from(failureContextIndexes)
      .sort((a, b) => a - b)
      .map((index) => ({
        kind: "kassiber.support_bundle.last_failure",
        schema_version: 1,
        index,
        record: events[index],
      })),
    ...aiIndexes.map((index) => ({
      kind: "kassiber.support_bundle.ai_provenance",
      schema_version: 1,
      index,
      record: events[index],
    })),
  ];

  return `${lines.map((line) => JSON.stringify(line)).join("\n")}\n`;
}

function supportBundleFormatNote(mode: AppLogRedactionMode): string {
  if (mode === "public_safe") {
    return "Each following JSONL row is independently redacted for public support: wallet/credential material is stripped and operational fields such as amounts, addresses, txids, paths, URLs, emails, IPs, labels, and onion hosts are masked.";
  }
  return "High-signal support bundles keep operational debugging data readable, including amounts, addresses, txids, paths, URLs, labels, emails, IPs, onion hosts, and error text. They still strip wallet/credential material and are intended for the maintainer or a trusted debugging session, not public posting.";
}

export function stableMaskedValue(field: AppLogField): string {
  const value = String(field.value ?? "");
  if (!value) return "";
  switch (field.type) {
    case "address":
      return keepShort(value, 5, 4);
    case "url":
      return `url#${stableHash(value)}`;
    case "xpub":
      return `xpub#${stableHash(value)}`;
    case "xpriv":
      return "[redacted-private-key]";
    case "descriptor":
      return maskDescriptor(value);
    case "txid":
      return `txid#${stableHash(value)}`;
    case "path":
      return maskPath(value);
    case "label":
      return `wallet#${stableHash(value)}`;
    case "onion":
      return `onion#${stableHash(value)}`;
    case "api_key":
      return "[redacted-api-key]";
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
  const mode = options.mode ?? "public_safe";
  if (SECRET_FLOOR_FIELD_TYPES.has(field.type)) {
    return { type: "text", value: stableMaskedValue(field) };
  }
  if (field.type === "text") {
    const value = stringifyLogValue(field.value);
    return { ...field, value: redactTextForMode(value, mode) };
  }
  if (field.type === "amount") {
    return mode === "public_safe" && options.maskAmounts
      ? { type: "text", value: stableMaskedValue(field) }
      : field;
  }
  if (OPERATIONAL_FIELD_TYPES.has(field.type)) {
    if (mode === "public_safe") {
      return { type: "text", value: stableMaskedValue(field) };
    }
    return { ...field, value: redactSecretFloorText(stringifyLogValue(field.value)) };
  }
  if (mode === "high_signal") {
    return field;
  }
  const value = stringifyLogValue(field.value);
  return { ...field, value: redactTextForMode(value, mode) };
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
  if (
    !SECRET_FLOOR_FIELD_TYPES.has(field.type) &&
    !OPERATIONAL_FIELD_TYPES.has(field.type)
  ) {
    return name;
  }
  if (
    field.type === "xpub" ||
    field.type === "xpriv" ||
    field.type === "descriptor"
  ) {
    return "wallet_material";
  }
  return field.type;
}

function secretFloorRecordAtInsert(record: AppLogRecord): AppLogRecord {
  return {
    ...record,
    msg: redactSecretFloorText(record.msg),
    fields: secretFloorFieldsAtInsert(record.fields),
    spantrace: record.spantrace?.map(secretFloorRecordAtInsert),
  };
}

function secretFloorFieldsAtInsert(
  fields: Record<string, AppLogField>,
): Record<string, AppLogField> {
  let changed = false;
  const entries = Object.entries(fields).map(([name, field]) => {
    if (field.type !== "text" || typeof field.value !== "string") {
      return [name, field] as const;
    }
    const value = redactSecretFloorText(field.value);
    if (value === field.value) return [name, field] as const;
    changed = true;
    return [name, { ...field, value }] as const;
  });
  return changed ? Object.fromEntries(entries) : fields;
}

function redactTextForMode(value: string, mode: AppLogRedactionMode): string {
  return mode === "public_safe"
    ? redactPublicSafeText(value)
    : redactSecretFloorText(value);
}

function redactSecretFloorText(value: string): string {
  return applyTextBackstop(value, SECRET_FLOOR_TEXT_PATTERNS);
}

function redactPublicSafeText(value: string): string {
  return applyTextBackstop(
    redactSecretFloorText(value),
    PUBLIC_SAFE_TEXT_PATTERNS,
  );
}

function applyTextBackstop(
  value: string,
  patterns: Array<[RegExp, TextBackstopReplacement]>,
): string {
  return patterns.reduce(
    (current, [pattern, replacement]) =>
      typeof replacement === "string"
        ? current.replace(pattern, replacement)
        : current.replace(pattern, replacement),
    value,
  );
}

function stringifyLogValue(value: unknown): string {
  if (
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function maskDescriptor(value: string): string {
  const scriptType = value.match(/\b(wpkh|sh|wsh|tr|pkh|combo)\(/i)?.[1];
  if (!scriptType) return "[redacted-descriptor]";
  const origin = value.match(/\[[^\]\n]{1,120}\]/)?.[0] ?? "";
  return `${scriptType}(${origin}[redacted-key])`;
}

function indexesForFailures(records: AppLogRecord[]): number[] {
  return records
    .map((record, index) => (isFailureRecord(record) ? index : -1))
    .filter((index) => index >= 0);
}

function isFailureRecord(record: AppLogRecord): boolean {
  if (record.level === "error") return true;
  if (record.fields.error_code) return true;
  return /(?:failed|threw|error)/i.test(record.msg);
}

function contextIndexesForFailures(
  records: AppLogRecord[],
  failureIndexes: number[],
  radius: number,
): Set<number> {
  const indexes = new Set<number>();
  for (const failureIndex of failureIndexes) {
    const traceId = stringFieldValue(records[failureIndex], "trace_id");
    const requestId = stringFieldValue(records[failureIndex], "request_id");
    records.forEach((record, index) => {
      if (
        traceId &&
        stringFieldValue(record, "trace_id") === traceId
      ) {
        indexes.add(index);
      }
      if (
        requestId &&
        stringFieldValue(record, "request_id") === requestId
      ) {
        indexes.add(index);
      }
    });
    for (
      let index = Math.max(0, failureIndex - radius);
      index <= Math.min(records.length - 1, failureIndex + radius);
      index += 1
    ) {
      indexes.add(index);
    }
  }
  return indexes;
}

function isAiProvenanceRecord(record: AppLogRecord): boolean {
  const kind = stringFieldValue(record, "kind");
  return (
    kind.startsWith("ai.chat") ||
    record.module.includes("ai") ||
    record.msg.includes("AI chat")
  );
}

function stringFieldValue(record: AppLogRecord, name: string): string {
  const field = record.fields[name];
  if (!field) return "";
  if (
    typeof field.value === "string" ||
    typeof field.value === "number" ||
    typeof field.value === "boolean"
  ) {
    return String(field.value);
  }
  return "";
}

function redactionReportForRecords(
  records: AppLogRecord[],
  omittedEvents: number,
  mode: AppLogRedactionMode = "public_safe",
): Record<string, unknown> {
  const secretFloorFieldCounts: Record<string, number> = {};
  const operationalFieldCounts: Record<string, number> = {};
  let secretFloorTextHits = 0;
  let publicSafeTextHits = 0;
  for (const record of records) {
    if (redactSecretFloorText(record.msg) !== record.msg) {
      secretFloorTextHits += 1;
    }
    if (mode === "public_safe" && redactPublicSafeText(record.msg) !== record.msg) {
      publicSafeTextHits += 1;
    }
    for (const field of Object.values(record.fields)) {
      if (SECRET_FLOOR_FIELD_TYPES.has(field.type)) {
        secretFloorFieldCounts[field.type] =
          (secretFloorFieldCounts[field.type] ?? 0) + 1;
      }
      if (OPERATIONAL_FIELD_TYPES.has(field.type) || field.type === "amount") {
        operationalFieldCounts[field.type] =
          (operationalFieldCounts[field.type] ?? 0) + 1;
      }
      const value = stringifyLogValue(field.value);
      if (redactSecretFloorText(value) !== value) {
        secretFloorTextHits += 1;
      }
      if (mode === "public_safe" && redactPublicSafeText(value) !== value) {
        publicSafeTextHits += 1;
      }
    }
  }
  return {
    mode,
    exact_amounts: mode === "public_safe" ? "masked" : "readable",
    omitted_events_from_start: omittedEvents,
    secret_floor_field_counts: secretFloorFieldCounts,
    operational_field_counts: operationalFieldCounts,
    secret_floor_text_hits: secretFloorTextHits,
    public_safe_text_hits: publicSafeTextHits,
    excluded_material: [
      "raw daemon arguments",
      "raw imported rows",
      "raw AI prompts",
      "database files",
      "descriptors",
      "xpubs",
      "private keys",
      "mnemonics",
      "backend URLs",
      "API keys",
      "local filesystem paths",
      "stack locals",
    ],
  };
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
