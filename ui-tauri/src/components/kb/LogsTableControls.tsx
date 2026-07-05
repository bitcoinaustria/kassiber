import {
  Eye,
  Regex,
  Settings,
  Shield,
  SlidersHorizontal,
  X,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  type AppLogLevel,
  type AppLogRecord,
} from "@/lib/appLogs";
import { cn } from "@/lib/utils";

const LEVEL_FILTER_ORDER: AppLogLevel[] = ["error", "warning", "info", "debug", "trace"];

export type LogLevelFilter = AppLogLevel | "all";

interface LogsTableControlsProps {
  hasTableFilters: boolean;
  levelFilter: LogLevelFilter;
  maskAmounts: boolean;
  moduleFilter: string | null;
  query: string;
  records: AppLogRecord[];
  redacted: boolean;
  regex: boolean;
  searchInputId: string;
  settingsActive: boolean;
  onClearFilters: () => void;
  onLevelFilterChange: (level: LogLevelFilter) => void;
  onMaskAmountsChange: (maskAmounts: boolean) => void;
  onModuleFilterChange: (module: string | null) => void;
  onQueryChange: (query: string) => void;
  onRedactedChange: (redacted: boolean) => void;
  onRegexChange: (regex: boolean) => void;
}

export function LogsTableControls({
  hasTableFilters,
  levelFilter,
  maskAmounts,
  moduleFilter,
  query,
  records,
  redacted,
  regex,
  searchInputId,
  settingsActive,
  onClearFilters,
  onLevelFilterChange,
  onMaskAmountsChange,
  onModuleFilterChange,
  onQueryChange,
  onRedactedChange,
  onRegexChange,
}: LogsTableControlsProps) {
  const { t } = useTranslation("review");
  const levelCounts = React.useMemo(() => countByLevel(records), [records]);
  const sortedModuleCounts = React.useMemo(
    () =>
      Object.entries(countByModule(records)).sort(([left], [right]) =>
        left.localeCompare(right),
      ),
    [records],
  );
  const trimmedQuery = query.trim();

  return (
    <div className="flex flex-wrap items-center gap-2 border-b p-3">
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
        {hasTableFilters ? (
          <>
            {levelFilter !== "all" ? (
              <ActiveFilterChip
                ariaLabel={t("logsControls.clearLevelAria", { level: levelFilter })}
                onClick={() => onLevelFilterChange("all")}
              >
                {t("logsControls.levelChip", { level: levelFilter.toUpperCase() })}
              </ActiveFilterChip>
            ) : null}
            {moduleFilter ? (
              <ActiveFilterChip
                ariaLabel={t("logsControls.clearModuleAria", { module: moduleFilter })}
                onClick={() => onModuleFilterChange(null)}
              >
                {t("logsControls.moduleChip", { module: moduleFilter })}
              </ActiveFilterChip>
            ) : null}
            {trimmedQuery ? (
              <ActiveFilterChip
                ariaLabel={t("logsControls.clearSearchAria")}
                onClick={() => {
                  onQueryChange("");
                  onRegexChange(false);
                }}
              >
                {t("logsControls.searchChip", {
                  query: `${regex ? t("logsControls.searchRegexPrefix") : ""}${trimmedQuery}`,
                })}
              </ActiveFilterChip>
            ) : null}
          </>
        ) : (
          <span className="font-mono text-xs text-muted-foreground">
            {t("logsControls.allCaptured")}
          </span>
        )}
      </div>

      <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
        <Input
          id={searchInputId}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder={t("logsControls.searchPlaceholder")}
          className="h-8 w-44 font-mono text-xs"
        />
        <LogFilterMenu
          hasTableFilters={hasTableFilters}
          levelCounts={levelCounts}
          levelFilter={levelFilter}
          moduleFilter={moduleFilter}
          recordsCount={records.length}
          sortedModuleCounts={sortedModuleCounts}
          onClearFilters={onClearFilters}
          onLevelFilterChange={onLevelFilterChange}
          onModuleFilterChange={onModuleFilterChange}
        />
        <LogSettingsMenu
          maskAmounts={maskAmounts}
          redacted={redacted}
          regex={regex}
          settingsActive={settingsActive}
          onMaskAmountsChange={onMaskAmountsChange}
          onRedactedChange={onRedactedChange}
          onRegexChange={onRegexChange}
        />
      </div>
    </div>
  );
}

function LogFilterMenu({
  hasTableFilters,
  levelCounts,
  levelFilter,
  moduleFilter,
  recordsCount,
  sortedModuleCounts,
  onClearFilters,
  onLevelFilterChange,
  onModuleFilterChange,
}: {
  hasTableFilters: boolean;
  levelCounts: Record<AppLogLevel, number>;
  levelFilter: LogLevelFilter;
  moduleFilter: string | null;
  recordsCount: number;
  sortedModuleCounts: [string, number][];
  onClearFilters: () => void;
  onLevelFilterChange: (level: LogLevelFilter) => void;
  onModuleFilterChange: (module: string | null) => void;
}) {
  const { t } = useTranslation("review");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          size="icon"
          variant={hasTableFilters ? "secondary" : "outline"}
          className={cn("relative size-8", hasTableFilters && "border-primary")}
          aria-label={t("logsControls.filterLogsAria")}
          title={t("logsControls.filterLogsAria")}
        >
          <SlidersHorizontal className="size-4" aria-hidden="true" />
          {hasTableFilters ? <ActiveDot /> : null}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel>{t("logsControls.filterLogsLabel")}</DropdownMenuLabel>
        <DropdownMenuCheckboxItem
          checked={levelFilter === "all"}
          onCheckedChange={() => onLevelFilterChange("all")}
        >
          <span>{t("logsControls.allLevels")}</span>
          <span className="ml-auto text-xs text-muted-foreground">{recordsCount}</span>
        </DropdownMenuCheckboxItem>
        {LEVEL_FILTER_ORDER.map((item) => (
          <DropdownMenuCheckboxItem
            key={item}
            checked={levelFilter === item}
            onCheckedChange={() => onLevelFilterChange(item)}
          >
            <span className="uppercase">{item}</span>
            <span className="ml-auto text-xs text-muted-foreground">
              {levelCounts[item]}
            </span>
          </DropdownMenuCheckboxItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuLabel>{t("logsControls.module")}</DropdownMenuLabel>
        <DropdownMenuCheckboxItem
          checked={moduleFilter === null}
          onCheckedChange={() => onModuleFilterChange(null)}
        >
          <span>{t("logsControls.allModules")}</span>
          <span className="ml-auto text-xs text-muted-foreground">{recordsCount}</span>
        </DropdownMenuCheckboxItem>
        {sortedModuleCounts.map(([module, count]) => (
          <DropdownMenuCheckboxItem
            key={module}
            checked={moduleFilter === module}
            onCheckedChange={() =>
              onModuleFilterChange(moduleFilter === module ? null : module)
            }
          >
            <span className="truncate font-mono">{module}</span>
            <span className="ml-auto text-xs text-muted-foreground">{count}</span>
          </DropdownMenuCheckboxItem>
        ))}
        {hasTableFilters ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={onClearFilters}>
              <X className="size-4" aria-hidden="true" />
              {t("logsControls.clearFilters")}
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function LogSettingsMenu({
  maskAmounts,
  redacted,
  regex,
  settingsActive,
  onMaskAmountsChange,
  onRedactedChange,
  onRegexChange,
}: {
  maskAmounts: boolean;
  redacted: boolean;
  regex: boolean;
  settingsActive: boolean;
  onMaskAmountsChange: (maskAmounts: boolean) => void;
  onRedactedChange: (redacted: boolean) => void;
  onRegexChange: (regex: boolean) => void;
}) {
  const { t } = useTranslation("review");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          size="icon"
          variant={settingsActive ? "secondary" : "outline"}
          className={cn("relative size-8", settingsActive && "border-primary")}
          aria-label={t("logsControls.logSettingsAria")}
          title={t("logsControls.logSettingsAria")}
        >
          <Settings className="size-4" aria-hidden="true" />
          {settingsActive ? <ActiveDot /> : null}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel>{t("logsControls.logSettingsLabel")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="font-mono text-xs text-muted-foreground">
          {t("logsControls.search")}
        </DropdownMenuLabel>
        <DropdownMenuCheckboxItem
          checked={regex}
          onCheckedChange={(checked) => onRegexChange(Boolean(checked))}
        >
          <Regex className="size-4" aria-hidden="true" />
          {t("logsControls.regexSearch")}
        </DropdownMenuCheckboxItem>
        <DropdownMenuSeparator />
        <DropdownMenuCheckboxItem
          checked={redacted}
          onCheckedChange={(checked) => onRedactedChange(Boolean(checked))}
        >
          <Shield className="size-4" aria-hidden="true" />
          {t("logsControls.redactedView")}
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem
          checked={maskAmounts}
          onCheckedChange={(checked) => onMaskAmountsChange(Boolean(checked))}
        >
          <Eye className="size-4" aria-hidden="true" />
          {t("logsControls.maskAmounts")}
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ActiveFilterChip({
  ariaLabel,
  children,
  onClick,
}: {
  ariaLabel: string;
  children: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="inline-flex h-7 max-w-full items-center gap-1 rounded-sm border border-primary/40 bg-primary/10 px-2 font-mono text-xs text-primary transition-colors hover:bg-primary/15"
      onClick={onClick}
      aria-label={ariaLabel}
    >
      <span className="truncate">{children}</span>
      <X className="size-3 shrink-0" aria-hidden="true" />
    </button>
  );
}

function ActiveDot() {
  return (
    <span
      className="absolute right-1 top-1 size-1.5 rounded-full bg-primary"
      aria-hidden="true"
    />
  );
}

function countByModule(records: AppLogRecord[]): Record<string, number> {
  return records.reduce<Record<string, number>>((acc, record) => {
    acc[record.module] = (acc[record.module] ?? 0) + 1;
    return acc;
  }, {});
}

function countByLevel(records: AppLogRecord[]): Record<AppLogLevel, number> {
  const counts: Record<AppLogLevel, number> = {
    trace: 0,
    debug: 0,
    info: 0,
    warning: 0,
    error: 0,
  };
  for (const record of records) {
    counts[record.level] += 1;
  }
  return counts;
}
