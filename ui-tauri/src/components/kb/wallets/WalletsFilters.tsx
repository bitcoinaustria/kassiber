import { SlidersHorizontal, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { connectionKindCategoryLabels } from "@/lib/connectionDisplay";
import { cn } from "@/lib/utils";
import type { ConnectionStatus } from "@/mocks/seed";

const kindFilterOptions = Array.from(
  new Set([...Object.values(connectionKindCategoryLabels), "Liquid"]),
);

const statusFilterOptions: ConnectionStatus[] = [
  "synced",
  "syncing",
  "idle",
  "error",
];

const filterChipClassName =
  "inline-flex items-center gap-1 rounded-md border bg-background px-2 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-muted sm:text-xs";

interface WalletsFiltersProps {
  filteredCount: number;
  hasActiveFilters: boolean;
  kindFilter: string | "all";
  onClearFilters: () => void;
  onKindFilterChange: (value: string | "all") => void;
  onStatusFilterChange: (value: ConnectionStatus | "all") => void;
  statusFilter: ConnectionStatus | "all";
}

export function WalletsFilters({
  filteredCount,
  hasActiveFilters,
  kindFilter,
  onClearFilters,
  onKindFilterChange,
  onStatusFilterChange,
  statusFilter,
}: WalletsFiltersProps) {
  const { t } = useTranslation(["connections", "chrome"]);
  const activeFilterCount =
    (statusFilter !== "all" ? 1 : 0) + (kindFilter !== "all" ? 1 : 0);

  return (
    <>
      <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5">
        <div className="flex flex-1 items-center gap-2">
          <span className="text-sm font-medium sm:text-base">
            {t("filters.heading")}
          </span>
          <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {filteredCount}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  activeFilterCount > 0 && "border-primary",
                )}
                aria-label={t("filters.menu")}
              >
                <SlidersHorizontal className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">{t("filters.menu")}</span>
                {activeFilterCount > 0 ? (
                  <span className="grid min-w-4 place-items-center rounded-full bg-primary px-1 text-[10px] font-semibold leading-4 text-primary-foreground">
                    {activeFilterCount}
                  </span>
                ) : null}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[220px]">
              <DropdownMenuLabel>{t("filters.menu")}</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("filters.status")}</span>
                  {statusFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[180px]">
                  <DropdownMenuRadioGroup
                    value={statusFilter}
                    onValueChange={(value) =>
                      onStatusFilterChange(value as ConnectionStatus | "all")
                    }
                  >
                    <DropdownMenuRadioItem value="all">
                      {t("filters.allStatuses")}
                    </DropdownMenuRadioItem>
                    {statusFilterOptions.map((status) => (
                      <DropdownMenuRadioItem key={status} value={status}>
                        {t(`chrome:connectionStatus.${status}`)}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("filters.kind")}</span>
                  {kindFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[200px]">
                  <DropdownMenuRadioGroup
                    value={kindFilter}
                    onValueChange={onKindFilterChange}
                  >
                    <DropdownMenuRadioItem value="all">
                      {t("filters.allKinds")}
                    </DropdownMenuRadioItem>
                    {kindFilterOptions.map((kind) => (
                      <DropdownMenuRadioItem key={kind} value={kind}>
                        {kind}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {hasActiveFilters && (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            {t("filters.label")}
          </span>
          {statusFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onStatusFilterChange("all")}
              aria-label={t("filters.clearStatus", {
                status: t(`chrome:connectionStatus.${statusFilter}`),
              })}
            >
              {t(`chrome:connectionStatus.${statusFilter}`)}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {kindFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onKindFilterChange("all")}
              aria-label={t("filters.clearKind", { kind: kindFilter })}
            >
              {kindFilter}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          <button
            type="button"
            onClick={onClearFilters}
            className="text-[10px] text-destructive hover:underline sm:text-xs"
          >
            {t("filters.clearAll")}
          </button>
        </div>
      )}
    </>
  );
}
