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
const AMOUNT_PSEUDONYM_SALT = createAmountPseudonymSalt();

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

// Named operational detectors. txid + amount are handled by the shared
// pseudonymizers below (both tiers); the rest are public_safe-only hard masks.
const PS_URL_RE = /\b(?:https?|tcp|ssl):\/\/[^\s,;"')\]}]+/gi;
const PS_EMAIL_RE = /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi;
const PS_RATE_RE = new RegExp(
  `\\b(?:${PUBLIC_SAFE_PAIR_UNITS})[/-](?:${PUBLIC_SAFE_PAIR_UNITS})\\s*(?::|=|at|rate)?\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`,
  "gi",
);
const PS_BECH32_RE = /\b(?:bc1|tb1|bcrt1|lq1|ex1)[023456789acdefghjklmnpqrstuvwxyz]{20,90}\b/gi;
const PS_BASE58_RE = /\b[13][1-9A-HJ-NP-Za-km-z]{25,34}\b/g;
const PS_ONION_RE = /\b[A-Za-z0-9.-]{16,}\.onion\b/gi;
const PS_PATH_RE =
  /(?:^|[\s"'(])(?:\/Users|\/home|\/var|\/private|\/tmp)\/[^\s,;"')\]}]+/g;

// One combined amount detector covering "UNIT NUM", "NUM UNIT", "SYM NUM" and
// "NUM SYM". A single .replace() pass means the pseudonym we emit (which itself
// contains a "~<magnitude> <unit>" string) is never re-scanned and double-masked.
// All inner groups are non-capturing, so the callback gets (match, offset, full).
const AMOUNT_TOKEN_RE = new RegExp(
  [
    `\\b(?:${PUBLIC_SAFE_AMOUNT_UNITS})\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`,
    `(?<![A-Za-z0-9])${PUBLIC_SAFE_AMOUNT_NUMBER}\\s*(?:${PUBLIC_SAFE_AMOUNT_UNITS})\\b`,
    `[${PUBLIC_SAFE_CURRENCY_SYMBOLS}]\\s*${PUBLIC_SAFE_AMOUNT_NUMBER}\\b`,
    `(?<![A-Za-z0-9])${PUBLIC_SAFE_AMOUNT_NUMBER}\\s*[${PUBLIC_SAFE_CURRENCY_SYMBOLS}]`,
  ].join("|"),
  "gi",
);
const MARKET_RATE_PAIR_PREFIX_RE = new RegExp(
  `(?:^|\\b)(?:${PUBLIC_SAFE_PAIR_UNITS})$`,
  "i",
);

function createAmountPseudonymSalt(): string {
  try {
    const bytes = new Uint32Array(2);
    globalThis.crypto?.getRandomValues?.(bytes);
    if (bytes.some((part) => part !== 0)) {
      return Array.from(bytes, (part) => part.toString(16).padStart(8, "0")).join("");
    }
  } catch {
    // Fall through to a non-cryptographic runtime nonce. The salt is not exported;
    // it only prevents offline dictionary reversal of low-entropy exact amounts.
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function isMarketRateTail(full: string, offset: number): boolean {
  const prev = full[offset - 1];
  if (prev !== "/" && prev !== "-") return false;
  return MARKET_RATE_PAIR_PREFIX_RE.test(full.slice(0, offset - 1).trimEnd());
}

function replaceAmounts(text: string, showScale: boolean): string {
  return text.replace(
    AMOUNT_TOKEN_RE,
    (match: string, offset: number, full: string) => {
      // A market rate ("BTC/EUR 64000.12") is public data, not the user's
      // amount — its trailing "EUR 64000.12" is preceded by the pair separator,
      // so leave it readable (public_safe masks the whole rate separately).
      if (isMarketRateTail(full, offset)) return match;
      const numMatch = match.match(/[+-]?\d[\d.,_ ]*\d|[+-]?\d/);
      const num = numMatch ? numMatch[0] : match;
      const unit = match.replace(num, "").replace(/\s+/g, "").trim();
      return pseudoAmount(num, unit, showScale);
    },
  );
}

// Glued/keyed sat amounts: amount_sat=50000, fee_msat: 100000, "value_sats":12345.
// The unit is part of the identifier, so AMOUNT_TOKEN_RE's \b never fires and the
// integer LOOKS protected while leaking. Match key=value / key:value where the
// key ends in a sat/msat unit and pseudonymize just the number, keeping the key.
const KEYED_AMOUNT_RE =
  /\b([A-Za-z][A-Za-z0-9_]*(?:msats?|sats?))\b(['"]?\s*[:=]\s*['"]?)([+-]?\d[\d.,_]*\d|[+-]?\d)/gi;

function replaceKeyedAmounts(text: string, showScale: boolean): string {
  return text.replace(
    KEYED_AMOUNT_RE,
    (_m: string, key: string, sep: string, num: string) => {
      const unit = /msat/i.test(key) ? "msat" : "sat";
      return `${key}${sep}${pseudoAmount(num, unit, showScale)}`;
    },
  );
}

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
    msg: redactTextForMode(record.msg, options),
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
    // The active filter embeds the user's raw log-search query, which can be a
    // txid/amount they pasted in to find a record; scrub it in redacted exports.
    `- Active filter: ${
      options.redacted
        ? redactTextForMode(options.header.activeFilter, options)
        : options.header.activeFilter
    }`,
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
      // The active filter carries the user's raw log-search query (possibly a
      // pasted txid/amount); pseudonymize it like any other operational text.
      active_filter: redactTextForMode(options.header.activeFilter, renderOptions),
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
      description: redactTextForMode(options.issueDescription.trim(), renderOptions),
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
    return "Each following JSONL row is independently redacted for public support: wallet/credential material is stripped, txids are replaced with stable pseudonyms (txid#…), amounts are replaced with salted runtime pseudonyms (amount#…), and operational fields such as addresses, paths, URLs, emails, IPs, labels, and onion hosts are masked.";
  }
  return "High-signal support bundles keep operational debugging data readable, including addresses, paths, URLs, labels, emails, IPs, onion hosts, and error text. Txids are replaced with stable pseudonyms (txid#…), and amounts are replaced with salted runtime pseudonyms (amount#… with a coarse ~magnitude) — never the raw value — so cross-line correlation survives without exposing exact amounts. Wallet/credential material is stripped. Intended for the maintainer or a trusted debugging session, not public posting.";
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
      return pseudoTxid(value);
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
    case "amount": {
      const { num, unit } = parseTypedAmount(value);
      return pseudoAmount(num, unit, false);
    }
    default:
      return value;
  }
}

function redactField(
  field: AppLogField,
  options: AppLogRenderOptions,
): AppLogField {
  const mode = options.mode ?? "public_safe";
  const showScale = !options.maskAmounts;
  if (SECRET_FLOOR_FIELD_TYPES.has(field.type)) {
    return { type: "text", value: stableMaskedValue(field) };
  }
  // txid + amount are pseudonymized in BOTH tiers — never raw in any redacted
  // export. The stable pseudonym preserves cross-line correlation; for amounts
  // a coarse magnitude is appended (unless scale is hidden) so sat/msat-scale
  // and fee-plausibility debugging still works.
  if (field.type === "txid") {
    return { type: "text", value: pseudoTxid(String(field.value ?? "")) };
  }
  if (field.type === "amount") {
    const { num, unit } = parseTypedAmount(field.value);
    return { type: "text", value: pseudoAmount(num, unit, showScale) };
  }
  if (field.type === "text") {
    const value = stringifyLogValue(field.value);
    return { ...field, value: redactTextForMode(value, options) };
  }
  if (OPERATIONAL_FIELD_TYPES.has(field.type)) {
    if (mode === "public_safe") {
      return { type: "text", value: stableMaskedValue(field) };
    }
    // high_signal keeps address/url/path/label/etc. readable (owner scope is
    // txid+amount only) but still runs the secret floor + txid/amount scrub.
    return {
      ...field,
      value: redactTextForMode(stringifyLogValue(field.value), options),
    };
  }
  if (mode === "high_signal") {
    // Number/boolean/duration values can't hide a txid/amount string, but a
    // STRING value mislabeled under a non-operational type can — scrub those
    // rather than passing them through verbatim.
    if (typeof field.value === "string") {
      return { ...field, value: redactTextForMode(field.value, options) };
    }
    return field;
  }
  const value = stringifyLogValue(field.value);
  return { ...field, value: redactTextForMode(value, options) };
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
    // Run the secret-shape backstop on every string value EXCEPT the declared
    // secret types: `redactField` masks those in both export tiers (and they
    // are only ever raw in the watermarked raw view), where the original is
    // needed to render a structure-revealing mask. Flooring here would still
    // miss a secret arriving under a non-secret label — a `url` carrying a
    // bearer token, or an unknown type that high-signal export passes through
    // verbatim — so those are covered.
    if (
      typeof field.value !== "string" ||
      SECRET_FLOOR_FIELD_TYPES.has(field.type)
    ) {
      return [name, field] as const;
    }
    const value = redactSecretFloorText(field.value);
    if (value === field.value) return [name, field] as const;
    changed = true;
    return [name, { ...field, value }] as const;
  });
  return changed ? Object.fromEntries(entries) : fields;
}

function redactTextForMode(value: string, options: AppLogRenderOptions): string {
  const mode = options.mode ?? "public_safe";
  const showScale = !options.maskAmounts;
  return mode === "public_safe"
    ? redactPublicSafeText(value, showScale)
    : redactHighSignalText(redactSecretFloorText(value), showScale);
}

function redactSecretFloorText(value: string): string {
  return applyTextBackstop(value, SECRET_FLOOR_TEXT_PATTERNS);
}

// high_signal keeps operational data (addresses, paths, URLs, labels) readable
// for debugging, but txids and amounts are ALWAYS pseudonymized — they are the
// wallet fingerprint an AI debugging session must never receive raw.
function redactHighSignalText(value: string, showScale: boolean): string {
  let out = value.replace(TXID_RE, (m) => pseudoTxid(m));
  out = replaceAmounts(out, showScale);
  // Keyed amounts last: replaceAmounts has already run, so the "~magnitude unit"
  // the keyed pseudonym emits is never re-scanned and double-masked.
  return replaceKeyedAmounts(out, showScale);
}

function redactPublicSafeText(value: string, showScale: boolean): string {
  let out = redactSecretFloorText(value);
  out = out.replace(PS_URL_RE, "[redacted-url]");
  out = out.replace(PS_EMAIL_RE, "[redacted-email]");
  out = out.replace(PS_RATE_RE, "[redacted-rate]");
  // txid + amount become stable pseudonyms (not [redacted]) so even a public
  // bundle keeps cross-line correlation; addresses/onion/paths stay hard-masked.
  out = out.replace(TXID_RE, (m) => pseudoTxid(m));
  out = replaceAmounts(out, showScale);
  out = replaceKeyedAmounts(out, showScale);
  out = out.replace(PS_BECH32_RE, "[redacted-address]");
  out = out.replace(PS_BASE58_RE, "[redacted-address]");
  out = out.replace(PS_ONION_RE, "[redacted-onion]");
  out = out.replace(PS_PATH_RE, (m) => `${m[0] === "/" ? "" : m[0]}[redacted-path]`);
  return out;
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
    if (mode === "public_safe" && redactPublicSafeText(record.msg, true) !== record.msg) {
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
      if (mode === "public_safe" && redactPublicSafeText(value, true) !== value) {
        publicSafeTextHits += 1;
      }
    }
  }
  return {
    mode,
    // txids + amounts are pseudonymized in BOTH tiers; high_signal additionally
    // keeps a coarse ~magnitude on amounts for debugging, public_safe drops it.
    txids: "pseudonymized",
    amounts: mode === "public_safe" ? "pseudonymized" : "pseudonymized-with-magnitude",
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
  // FNV-1a (32-bit). The daemon mirrors this exactly in
  // kassiber/redaction.py::_stable_hash so a txid pseudonymized Python-side and
  // one pseudonymized here collapse to the SAME token across the merged stream.
  // 8 hex chars (32-bit space) keeps cross-line correlation collision-safe on a
  // full bundle; the old 4-hex truncation collided after a few thousand values.
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

// txids and amounts are NEVER readable in an export (any tier) — they are the
// wallet-fingerprinting data a developer must not hand to an AI after a
// real-wallet test sync. Txids stay globally stable for correlation; amount
// tokens are salted so low-entropy exact amounts cannot be dictionary-reversed.
// {64,} (not {64}) so a >=65-hex run (two concatenated txids, a txid glued to
// trailing hex) still has a word boundary and is pseudonymized as one token
// instead of slipping through the {64}\b gap.
const TXID_RE = /\b[0-9a-f]{64,}\b/gi;

function pseudoTxid(value: string): string {
  return `txid#${stableHash(value.toLowerCase())}`;
}

function parseAmountNumber(raw: string): number {
  // Logs are machine/English (CLI/daemon are not localized), so `.` is the
  // decimal separator; strip `,`/`_`/space grouping before parsing.
  return Number.parseFloat(raw.replace(/[,_ ]/g, ""));
}

// Coarse order-of-magnitude bucket, e.g. 0.0123 -> "~0.01", 12345678 -> "~1e7".
// Keeps debugging signal (sat/msat scale, fee plausibility) without the exact,
// fingerprinting value.
function amountMagnitude(raw: string): string {
  const n = parseAmountNumber(raw);
  if (!Number.isFinite(n) || n === 0) return "~0";
  const exp = Math.floor(Math.log10(Math.abs(n)));
  const sign = n < 0 ? "-" : "";
  let body: string;
  if (exp < 0) body = Math.pow(10, exp).toFixed(-exp);
  else if (exp <= 4) body = String(Math.pow(10, exp));
  else body = `1e${exp}`;
  return `~${sign}${body}`;
}

function pseudoAmount(rawNumber: string, unit: string, showScale: boolean): string {
  // Include a runtime-only salt so low-entropy exact amounts cannot be recovered
  // by enumerating likely values against a public `amount#...` token. The raw
  // amount still normalizes as a string to avoid float-formatting drift.
  const key = `${AMOUNT_PSEUDONYM_SALT}|${rawNumber.replace(/[,_ ]/g, "")}|${unit.toLowerCase()}`;
  const token = `amount#${stableHash(key)}`;
  if (!showScale) return token;
  const mag = amountMagnitude(rawNumber);
  return unit ? `${token} (${mag} ${unit})` : `${token} (${mag})`;
}

// Split a typed `amount` field value (e.g. "1.234 BTC", "2500 sats", or a bare
// number) into its numeric and unit parts for pseudonymization.
function parseTypedAmount(value: unknown): { num: string; unit: string } {
  const text = typeof value === "number" ? String(value) : String(value ?? "").trim();
  const match = text.match(
    /^([+-]?[\d.,_ ]*\d)\s*([A-Za-z]{1,6}|[€$£¥₿])?/,
  );
  if (!match) return { num: text, unit: "" };
  return { num: match[1].trim(), unit: (match[2] ?? "").trim() };
}
