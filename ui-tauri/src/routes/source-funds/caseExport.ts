import { sourceFundsExportArgs } from "@/lib/sourceFundsExport";
import type { SourceFundsPreview } from "./model";

/** Each awaited step must still belong to the reviewed recipe and mounted book. */
export async function exportCurrentCase({ save, render, isCurrent }: {
  save: () => Promise<SourceFundsPreview | undefined>;
  render: (args: { case: string }) => Promise<{ filename?: string } | undefined>;
  isCurrent: () => boolean;
}): Promise<{ savedCase: NonNullable<SourceFundsPreview["case"]>; output?: { filename?: string } } | null> {
  if (!isCurrent()) return null;
  const saved = await save();
  if (!isCurrent()) return null;
  const args = sourceFundsExportArgs(saved);
  if (!args || !saved?.case) return null;
  const output = await render(args);
  if (!isCurrent()) return null;
  return { savedCase: saved.case, output };
}
