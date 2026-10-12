import { FileSpreadsheet, FileText, Tags, type LucideIcon } from "lucide-react";

import { CONNECTION_SOURCES } from "@/lib/connectionCatalog";

/**
 * Transaction export formats offered by the Export dialog.
 *
 * Behavior lives in the core (`kassiber/core/transaction_formats.py` for
 * other tools' import formats); this is only how a format is shown. A tool
 * format reuses its connection-catalog entry (same `id`) for its logo, so a
 * tool looks the same where Kassiber imports from it and where it exports to
 * it. Adding a tool is one core format plus one entry here.
 */
export type ExportFormatGroup = "kassiber" | "tool";

/** Every format id; each needs `formats.<id>` and `formatHints.<id>` strings. */
export type ExportFormatId = "xlsx" | "csv" | "labels" | "cointracking" | "blockpit";
/** Formats for another tool; each also needs an `importHint.<id>` string. */
export type ToolFormatId = Exclude<ExportFormatId, "xlsx" | "csv" | "labels">;

export interface ExportFormatOption {
  id: ExportFormatId;
  group: ExportFormatGroup;
  /** File type badge, e.g. "XLSX". */
  extension: string;
  /** Daemon export: the Kassiber workbook/CSV kinds, BIP 329 labels, or a platform id. */
  daemon: { kind: "xlsx" | "csv" | "labels" } | { kind: "platform"; platform: string };
  /** False when the file is not per transaction date (labels). */
  dated: boolean;
  defaultName: string;
  filter: { name: string; extensions: string[] };
  /** Lucide icon for Kassiber's own file types. */
  icon?: LucideIcon;
  /** Tile color for the icon. */
  tone?: string;
  /** Connection-catalog id whose logo identifies the tool. */
  catalogId?: string;
}

export const EXPORT_FORMATS: ExportFormatOption[] = [
  {
    id: "xlsx",
    group: "kassiber",
    extension: "XLSX",
    daemon: { kind: "xlsx" },
    defaultName: "kassiber-transactions.xlsx",
    filter: { name: "Excel workbook", extensions: ["xlsx"] },
    icon: FileSpreadsheet,
    tone: "bg-emerald-600 text-white",
    dated: true,
  },
  {
    id: "csv",
    group: "kassiber",
    extension: "CSV",
    daemon: { kind: "csv" },
    defaultName: "kassiber-transactions.csv",
    filter: { name: "CSV", extensions: ["csv"] },
    icon: FileText,
    tone: "bg-slate-600 text-white",
    dated: true,
  },
  {
    id: "labels",
    group: "kassiber",
    extension: "JSONL",
    daemon: { kind: "labels" },
    defaultName: "kassiber-labels.jsonl",
    filter: { name: "BIP 329 labels", extensions: ["jsonl"] },
    icon: Tags,
    tone: "bg-violet-600 text-white",
    dated: false,
  },
  {
    id: "cointracking",
    group: "tool",
    extension: "CSV",
    daemon: { kind: "platform", platform: "cointracking" },
    defaultName: "kassiber-cointracking.csv",
    filter: { name: "CSV", extensions: ["csv"] },
    catalogId: "cointracking",
    dated: true,
  },
  {
    id: "blockpit",
    group: "tool",
    extension: "CSV",
    daemon: { kind: "platform", platform: "blockpit" },
    defaultName: "kassiber-blockpit.csv",
    filter: { name: "CSV", extensions: ["csv"] },
    catalogId: "blockpit",
    dated: true,
  },
];

export const EXPORT_FORMAT_GROUPS: ExportFormatGroup[] = ["kassiber", "tool"];

export function exportFormat(id: ExportFormatId): ExportFormatOption {
  return EXPORT_FORMATS.find((option) => option.id === id) ?? EXPORT_FORMATS[0];
}

export function isToolFormat(option: ExportFormatOption): option is ExportFormatOption & {
  id: ToolFormatId;
} {
  return option.group === "tool";
}

/** Logo for a tool format, from the connection catalog; undefined for Kassiber files. */
export function exportFormatLogo(option: ExportFormatOption): string | undefined {
  if (!option.catalogId) return undefined;
  return CONNECTION_SOURCES.find((source) => source.id === option.catalogId)?.image;
}
