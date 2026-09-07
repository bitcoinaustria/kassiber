import { isFileSaveAvailable } from "@/lib/filePicker";
import { analysisCsv, type AnalysisResult } from "./chainAnalysis";

export async function exportChainAnalysis(
  result: AnalysisResult,
  format: "json" | "csv",
  title: string,
): Promise<"saved" | "download" | "cancelled"> {
  const filename = `kassiber-chain-analysis-${result.snapshot_id.slice(0, 16).replace(/[^a-zA-Z0-9_-]/g, "")}.${format}`;
  const contents =
    format === "json"
      ? JSON.stringify(result, null, 2) + "\n"
      : analysisCsv(result);
  if (isFileSaveAvailable) {
    const { invoke } = await import("@tauri-apps/api/core");
    // The native dialog owns the destination grant; renderer text cannot name a path.
    const destination = await invoke<string | null>(
      "save_chain_analysis_export_as",
      {
        contents,
        format,
        defaultName: filename,
        title,
      },
    );
    return destination ? "saved" : "cancelled";
  }
  const url = URL.createObjectURL(
    new Blob([contents], {
      type:
        format === "json"
          ? "application/json;charset=utf-8"
          : "text/csv;charset=utf-8",
    }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 30_000);
  return "download";
}
