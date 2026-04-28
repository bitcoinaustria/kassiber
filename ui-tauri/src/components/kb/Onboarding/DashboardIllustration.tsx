import { ChevronLeft, WalletCards } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

import { BACKEND_KIND_LABELS } from "./constants";
import type { OnboardingForm } from "./types";

interface DashboardIllustrationProps {
  form: OnboardingForm;
  variant?: "zoomed-in" | "zoomed-out";
  transformOrigin?: string;
}

/**
 * Static dashboard mockup shown on the right side of the first two steps.
 * The "zoom" between steps is a CSS transform with a tasteful ease-out
 * curve — accurate spring physics aren't worth the bundle weight of a
 * separate animation library for one decorative scale.
 */
export const DashboardIllustration = ({
  form,
  variant = "zoomed-out",
  transformOrigin = "-20% -10%",
}: DashboardIllustrationProps) => {
  const workspace = form.workspace.trim() || "Personal";
  const profile = form.profile.trim() || "main";
  const scaleClass = variant === "zoomed-in" ? "scale-[1.3]" : "scale-100";
  return (
    <div
      style={{ transformOrigin }}
      className={cn(
        "flex h-full min-h-[520px] w-[980px] overflow-hidden rounded-lg border border-line bg-paper shadow-sm transition-transform duration-500 ease-out motion-reduce:transition-none",
        scaleClass,
      )}
    >
      <div className="h-full w-[280px] shrink-0 overflow-hidden bg-paper-2">
        <div className="flex items-center justify-between gap-2 border-b border-line p-4">
          <div className="flex min-w-0 items-center gap-2">
            <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-ink text-paper">
              <WalletCards className="size-4" />
            </div>
            <div className="min-w-0">
              <p className="truncate font-semibold text-ink">{workspace}</p>
              <p className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                {profile}
              </p>
            </div>
          </div>
          <ChevronLeft className="size-4 text-ink-3" />
        </div>
        <ul className="space-y-2 p-4">
          {[
            "Overview",
            "Connections",
            "Transactions",
            "Reports",
            "Profiles",
          ].map((item, index) => (
            <li
              key={item}
              className={cn(
                "rounded-md border px-3 py-2 text-xs",
                index === 0
                  ? "border-ink bg-paper text-ink"
                  : "border-line bg-paper/70 text-ink-2",
              )}
            >
              {item}
            </li>
          ))}
        </ul>
      </div>
      <div className="flex min-w-0 flex-1 flex-col justify-between p-4">
        <div className="space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="space-y-1">
              <div className="h-4 w-40 rounded-md bg-ink" />
              <div className="h-3 w-64 rounded-md bg-line-2" />
            </div>
            <Button variant="outline" size="sm">
              Add wallet
            </Button>
          </div>

          <div className="grid grid-cols-4 gap-3">
            {[
              ["Policy", form.taxCountry === "at" ? "Austria" : "Generic"],
              ["Currency", form.fiatCurrency],
              [
                "Backend",
                form.backendSetupMode === "skip"
                  ? "Skipped"
                  : form.backendSetupMode === "custom"
                    ? BACKEND_KIND_LABELS[form.backendKind]
                    : "Built-ins",
              ],
              [
                "Database",
                form.databaseMode === "sqlcipher" ? "SQLCipher" : "Plain",
              ],
            ].map(([label, value]) => (
              <div key={label} className="rounded-lg border border-line p-3">
                <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                  {label}
                </div>
                <div className="mt-2 text-lg font-semibold text-ink">
                  {value}
                </div>
              </div>
            ))}
          </div>

          <div className="grid grid-cols-4 gap-3">
            {[
              [
                "Endpoint",
                form.backendSetupMode === "custom"
                  ? form.backendName || "custom"
                  : form.backendSetupMode === "skip"
                    ? "none"
                    : "mempool",
              ],
              [
                "Sync",
                form.backendSetupMode === "skip" ? "manual import" : "enabled",
              ],
              ["Secrets", "encrypted path"],
              [
                "Assistant",
                form.aiSetupMode === "disabled"
                  ? "disabled"
                  : form.aiSetupMode === "remote"
                    ? "remote/TEE"
                    : "local",
              ],
            ].map(([label, value]) => (
              <div key={label} className="rounded-lg border border-line p-3">
                <div className="font-mono text-[10px] uppercase tracking-[0.12em] text-ink-3">
                  {label}
                </div>
                <div className="mt-2 text-lg font-semibold text-ink">
                  {value}
                </div>
              </div>
            ))}
          </div>

          <div className="overflow-hidden rounded-lg border border-line">
            <Table>
              <TableHeader>
                <TableRow className="bg-paper-2">
                  {["Source", "Asset", "Status", "Scope"].map((head) => (
                    <TableHead
                      key={head}
                      className="h-9 border-r last:border-r-0"
                    >
                      {head}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {[
                  ["Treasury", "BTC", "watch-only", "local"],
                  ["BTCPay", "BTC", "credentials encrypted", "profile"],
                  ["Liquid", "LBTC", "manual pair", "audit"],
                  ["Reports", form.fiatCurrency, form.gainsAlgorithm, "tax"],
                ].map((row) => (
                  <TableRow
                    key={row.join("-")}
                    className="even:bg-paper-2/60"
                  >
                    {row.map((cell) => (
                      <TableCell
                        key={cell}
                        className="h-10 border-r last:border-r-0"
                      >
                        {cell}
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {["Sync", "Journal", "Report"].map((item) => (
            <div
              key={item}
              className="rounded-md border border-line bg-paper-2 px-3 py-2 text-xs text-ink-2"
            >
              {item}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};
