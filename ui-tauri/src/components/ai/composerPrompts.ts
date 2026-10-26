import { AlertTriangle, FileSpreadsheet, RefreshCw } from "lucide-react";

/** Starter questions offered by an empty composer (`assistant:prompts.*`). */
export const DEFAULT_COMPOSER_PROMPT_KEYS = [
  { icon: AlertTriangle, key: "reviewQuarantine" },
  { icon: RefreshCw, key: "reprocessJournals" },
  { icon: FileSpreadsheet, key: "prepareTaxExport" },
] as const;
