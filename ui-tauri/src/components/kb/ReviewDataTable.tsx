import {
  ArrowDownUp,
  ArrowRight,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock3,
  FileSearch,
  Search,
  ShieldAlert,
  X,
} from "lucide-react";
import { Link } from "@tanstack/react-router";
import {
  useEffect,
  useMemo,
  useState,
  type ComponentType,
  type ReactNode,
  type SVGProps,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

import {
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

export type ReviewTableKind = "journal-events" | "quarantine";

export interface ReviewTableRow {
  id: string;
  rowKey?: string;
  date: string;
  account: string;
  event: string;
  source: string;
  amount: string;
  basis: string;
  impact: string;
  status: "Ready" | "Needs review" | "Blocked" | "Resolved";
  priority: "Low" | "Medium" | "High";
  owner: string;
  evidenceHint?: string;
  nextAction?: string;
  metricFilterIds?: string[];
  transactionAction?: {
    transactionId: string;
    label: string;
    tab?: "details" | "classify" | "pricing" | "tax" | "linked" | "ledger";
    reviewReason?: string;
  };
}

type ReviewTransactionAction = NonNullable<ReviewTableRow["transactionAction"]>;

export function reviewRowKey(row: ReviewTableRow) {
  return row.rowKey ?? row.id;
}

export interface ReviewMetric {
  label: string;
  value: number | string;
  tone: ReviewTone;
  filterId?: string;
  filterLabel?: string;
}

interface ReviewDataTableProps {
  kind: ReviewTableKind;
  eyebrow: string;
  title: string;
  description: string;
  icon?: ComponentType<SVGProps<SVGSVGElement>>;
  rows: ReviewTableRow[];
  actions?: ReactNode;
  metrics?: ReviewMetric[];
  tableTitle?: string;
  tableDescription?: string;
  tableDescriptionDetail?: string;
  searchPlaceholder?: string;
  emptyMessage?: string;
  badgeLabel?: string;
  showSummaryBadge?: boolean;
  showStateColumn?: boolean;
  showPriorityBadge?: boolean;
  shellClassName?: string;
  onOpenTransactionAction?: (
    action: ReviewTransactionAction,
    row: ReviewTableRow,
  ) => void;
  /**
   * Reports the rows in the order they are actually shown (after search,
   * status/metric filters, and sort). Lets a parent drive queue navigation
   * (e.g. "Save & next") against what the user sees, not the raw input order.
   */
  onVisibleRowsChange?: (rows: ReviewTableRow[]) => void;
}

const statusClass: Record<ReviewTableRow["status"], string> = {
  Ready:
    "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/50 dark:bg-emerald-950/40 dark:text-emerald-300",
  "Needs review":
    "border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900/50 dark:bg-amber-950/40 dark:text-amber-300",
  Blocked:
    "border-red-200 bg-red-50 text-red-700 dark:border-red-900/50 dark:bg-red-950/40 dark:text-red-300",
  Resolved: "border-border bg-muted text-muted-foreground",
};

const priorityClass: Record<ReviewTableRow["priority"], string> = {
  Low: "bg-muted text-muted-foreground",
  Medium: "bg-secondary text-secondary-foreground",
  High: "bg-primary text-primary-foreground",
};

const statusTone: Record<ReviewTableRow["status"], ReviewTone> = {
  Ready: "good",
  "Needs review": "warning",
  Blocked: "alert",
  Resolved: "neutral",
};

const statusIcon: Record<
  ReviewTableRow["status"],
  ComponentType<SVGProps<SVGSVGElement>>
> = {
  Ready: CheckCircle2,
  "Needs review": Clock3,
  Blocked: ShieldAlert,
  Resolved: CheckCircle2,
};

const statusOptions: Array<ReviewTableRow["status"] | "All"> = [
  "All",
  "Needs review",
  "Blocked",
  "Resolved",
  "Ready",
];

const statusLabelKey = {
  All: "review.status.all",
  Ready: "review.status.ready",
  "Needs review": "review.status.needsReview",
  Blocked: "review.status.blocked",
  Resolved: "review.status.resolved",
} as const satisfies Record<ReviewTableRow["status"] | "All", string>;

const priorityLabelKey = {
  Low: "review.priority.low",
  Medium: "review.priority.medium",
  High: "review.priority.high",
} as const satisfies Record<ReviewTableRow["priority"], string>;

export type ReviewTone = "good" | "warning" | "alert" | "neutral";
type SortDirection = "desc" | "asc";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function ReviewDataTable({
  kind,
  eyebrow,
  title,
  description,
  icon: Icon,
  rows,
  actions,
  metrics,
  tableTitle,
  tableDescription,
  tableDescriptionDetail,
  searchPlaceholder,
  emptyMessage,
  badgeLabel,
  showSummaryBadge = true,
  showStateColumn = true,
  showPriorityBadge = true,
  shellClassName = screenShellClassName,
  onOpenTransactionAction,
  onVisibleRowsChange,
}: ReviewDataTableProps) {
  const { t } = useTranslation(["journals", "common"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [globalFilter, setGlobalFilter] = useState("");
  const [statusFilter, setStatusFilter] =
    useState<ReviewTableRow["status"] | "All">("All");
  const [metricFilterId, setMetricFilterId] = useState<string | null>(null);
  const [sortDirection, setSortDirection] = useState<SortDirection>("desc");
  const [pageIndex, setPageIndex] = useState(0);

  const activeRows = rows.filter((row) => row.status !== "Resolved");
  const resolvedCount = rows.length - activeRows.length;
  const blockedCount = rows.filter((row) => row.status === "Blocked").length;
  const reviewCount = rows.filter((row) => row.status === "Needs review").length;
  const highCount = rows.filter((row) => row.priority === "High").length;
  const queueTone: ReviewTone =
    blockedCount > 0 ? "alert" : reviewCount > 0 ? "warning" : "good";
  const filteredRows = useMemo(() => {
    const query = globalFilter.trim().toLowerCase();
    const filtered = rows.filter((row) => {
      const matchesMetric =
        !metricFilterId || row.metricFilterIds?.includes(metricFilterId);
      const matchesStatus =
        statusFilter === "All" || row.status === statusFilter;
      const matchesQuery =
        !query ||
        [
          row.id,
          row.rowKey ?? "",
          row.transactionAction?.transactionId ?? "",
          row.date,
          row.account,
          row.event,
          row.source,
          row.amount,
          row.basis,
          row.impact,
          row.status,
          row.priority,
          row.owner,
          row.evidenceHint ?? "",
          row.nextAction ?? "",
        ]
          .join(" ")
          .toLowerCase()
          .includes(query);
      return matchesMetric && matchesStatus && matchesQuery;
    });
    return filtered.sort((a, b) =>
      sortDirection === "desc"
        ? b.date.localeCompare(a.date)
        : a.date.localeCompare(b.date),
    );
  }, [globalFilter, metricFilterId, rows, sortDirection, statusFilter]);
  useEffect(() => {
    onVisibleRowsChange?.(filteredRows);
  }, [filteredRows, onVisibleRowsChange]);
  const pageSize = 8;
  const pageCount = Math.max(Math.ceil(filteredRows.length / pageSize), 1);
  const currentPage = Math.min(pageIndex, pageCount - 1);
  const pageRows = filteredRows.slice(
    currentPage * pageSize,
    currentPage * pageSize + pageSize,
  );
  const hasActiveFilters =
    metricFilterId !== null || statusFilter !== "All" || Boolean(globalFilter);
  const metricsToShow =
    metrics ??
    [
      { label: t("review.metricFallback.open"), value: activeRows.length, tone: queueTone },
      {
        label: t("review.metricFallback.needsReview"),
        value: reviewCount,
        tone: "warning",
      },
      {
        label: t("review.metricFallback.blocked"),
        value: blockedCount,
        tone: "alert",
      },
      {
        label: t("review.metricFallback.highPriority"),
        value: highCount,
        tone: highCount ? "alert" : "neutral",
      },
    ];
  const visibleMetricCount = Math.min(metricsToShow.length, 5);
  const activeMetric = metricFilterId
    ? metricsToShow.find((metric) => metric.filterId === metricFilterId)
    : null;
  const renderedTableDescription = tableDescriptionDetail
    ? t("review.tableDescription.detail", {
        rows: filteredRows.length,
        detail: tableDescriptionDetail,
      })
    : tableDescription ??
      t("review.tableDescription.resolved", {
        rows: filteredRows.length,
        resolved: resolvedCount,
      });

  const updateStatusFilter = (status: ReviewTableRow["status"] | "All") => {
    setStatusFilter(status);
    setPageIndex(0);
  };

  const updateMetricFilter = (filterId: string) => {
    setMetricFilterId(filterId === "all" ? null : filterId);
    setPageIndex(0);
  };

  const updateGlobalFilter = (value: string) => {
    setGlobalFilter(value);
    setPageIndex(0);
  };

  return (
    <div className={cn(shellClassName)}>
      <div className={pageHeaderClassName}>
        <div className="flex min-w-0 items-start gap-3">
          {Icon ? (
            <span
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
                toneStyles[queueTone],
              )}
              aria-hidden="true"
            >
              <Icon className="size-4" />
            </span>
          ) : null}
          <div className="min-w-0">
            <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
              {eyebrow}
            </p>
            <h1 className="mt-0.5 text-base font-semibold">{title}</h1>
            <p className="mt-0.5 max-w-4xl text-xs text-muted-foreground sm:text-sm">
              {description}
            </p>
          </div>
        </div>
        <div className={cn(pageHeaderActionsClassName, "shrink-0")}>
          {actions}
          {showSummaryBadge ? (
            <Badge
              variant="outline"
              className={cn("self-start rounded-md", toneBadgeStyles[queueTone])}
            >
              {badgeLabel ?? t("review.badgeOpen", { count: activeRows.length })}
            </Badge>
          ) : null}
        </div>
      </div>

      <div className="overflow-hidden rounded-lg border bg-card">
        <div
          className={cn(
            "grid grid-cols-2 divide-x-0 divide-y divide-border sm:divide-x sm:divide-y-0",
            visibleMetricCount >= 5 ? "sm:grid-cols-5" : "sm:grid-cols-4",
          )}
        >
          {metricsToShow.slice(0, 5).map((metric) => (
            <QueueMetric
              key={metric.label}
              label={metric.label}
              value={metric.value}
              tone={metric.tone}
              filterId={metric.filterId}
              active={
                metric.filterId === "all"
                  ? metricFilterId === null
                  : metric.filterId === metricFilterId
              }
              onFilter={updateMetricFilter}
            />
          ))}
        </div>
      </div>

      <div className="rounded-lg border bg-card">
        <div className="flex flex-col gap-3 border-b p-3 lg:flex-row lg:items-center lg:justify-between sm:px-4">
          <div className="flex min-w-0 items-center gap-2">
            <FileSearch
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <div className="min-w-0">
              <h2 className="text-sm font-medium sm:text-base">
                {tableTitle ??
                  (kind === "journal-events"
                    ? t("review.tableTitle.review")
                    : t("review.tableTitle.blocked"))}
              </h2>
              <p className="text-[10px] text-muted-foreground sm:text-xs">
                {renderedTableDescription}
              </p>
            </div>
          </div>
          <div className="flex flex-col gap-2 sm:flex-row">
            <div className="relative min-w-0 sm:w-80">
              <Search
                className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                value={globalFilter}
                onChange={(event) => updateGlobalFilter(event.target.value)}
                placeholder={searchPlaceholder ?? t("review.searchPlaceholder")}
                className="h-8 pl-9"
              />
            </div>
            <div className="flex flex-wrap gap-2">
              {showStateColumn ? statusOptions
                .filter(
                  (status) =>
                    status === "All" ||
                    rows.some((row) => row.status === status),
                )
                .map((status) => (
                  <Button
                    key={status}
                    type="button"
                    size="sm"
                    variant={statusFilter === status ? "default" : "outline"}
                    className="h-8"
                    onClick={() => updateStatusFilter(status)}
                  >
                    {t(statusLabelKey[status])}
                  </Button>
                )) : null}
            </div>
          </div>
        </div>

        {hasActiveFilters ? (
          <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2 sm:px-4">
            <span className="text-[10px] text-muted-foreground sm:text-xs">
              {t("review.filters.active")}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
              onClick={() => {
                setMetricFilterId(null);
                updateStatusFilter("All");
                updateGlobalFilter("");
              }}
            >
              {t("review.filters.clearAll")}
            </Button>
            {activeMetric ? (
              <button
                type="button"
                className="inline-flex h-7 items-center gap-1.5 rounded-md border bg-background px-2 text-xs text-foreground transition-colors hover:bg-muted"
                onClick={() => updateMetricFilter("all")}
                aria-label={t("review.filters.clearMetricAria", {
                  label: activeMetric.filterLabel ?? activeMetric.label,
                })}
              >
                {activeMetric.filterLabel ?? activeMetric.label}
                <X className="size-3" aria-hidden="true" />
              </button>
            ) : null}
          </div>
        ) : null}

        {kind === "quarantine" ? (
          <QuarantineReviewList
            rows={pageRows}
            emptyMessage={emptyMessage ?? t("review.empty")}
            hideSensitive={hideSensitive}
            showStateColumn={showStateColumn}
            showPriorityBadge={showPriorityBadge}
            sortDirection={sortDirection}
            onSortDirectionChange={() =>
              setSortDirection((value) => (value === "desc" ? "asc" : "desc"))
            }
            onOpenTransactionAction={onOpenTransactionAction}
          />
        ) : (
          <div className="overflow-x-auto">
            <Table className="min-w-[940px]">
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead className="min-w-[330px]">
                    {t("review.column.event")}
                  </TableHead>
                  <TableHead className="min-w-[180px]">
                    {t("review.column.evidence")}
                  </TableHead>
                  <TableHead className="min-w-[160px] text-right">
                    {t("review.column.amountBasis")}
                  </TableHead>
                  <TableHead className="min-w-[140px] text-right">
                    {t("review.column.impact")}
                  </TableHead>
                  {showStateColumn ? (
                    <TableHead className="min-w-[170px]">
                      {t("common:field.status")}
                    </TableHead>
                  ) : null}
                  <TableHead className="w-[112px] text-right">
                    <SortButton
                      label={t("common:field.date")}
                      direction={sortDirection}
                      onClick={() =>
                        setSortDirection((value) =>
                          value === "desc" ? "asc" : "desc",
                        )
                      }
                    />
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pageRows.length ? (
                  pageRows.map((row) => (
                    <ReviewWorklistRow
                      key={reviewRowKey(row)}
                      row={row}
                      hideSensitive={hideSensitive}
                      showStateColumn={showStateColumn}
                      showPriorityBadge={showPriorityBadge}
                      onOpenTransactionAction={onOpenTransactionAction}
                    />
                  ))
                ) : (
                  <TableRow>
                    <TableCell
                      colSpan={showStateColumn ? 6 : 5}
                      className="h-24 text-center text-muted-foreground"
                    >
                      {emptyMessage ?? t("review.empty")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}

        <div className="flex flex-col gap-3 border-t px-3 py-2.5 text-xs text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:px-4">
          <span>
            {filteredRows.length === 0
              ? t("review.pagination.noRows")
              : t("review.pagination.range", {
                  from: currentPage * pageSize + 1,
                  to: Math.min(
                    currentPage * pageSize + pageSize,
                    filteredRows.length,
                  ),
                  total: filteredRows.length,
                })}
          </span>
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="icon"
              className="size-7"
              onClick={() => setPageIndex((value) => Math.max(value - 1, 0))}
              disabled={currentPage === 0}
              aria-label={t("review.pagination.previous")}
            >
              <ChevronLeft className="size-4" aria-hidden="true" />
            </Button>
            <span>
              {t("review.pagination.page", {
                current: currentPage + 1,
                count: pageCount,
              })}
            </span>
            <Button
              variant="outline"
              size="icon"
              className="size-7"
              onClick={() =>
                setPageIndex((value) => Math.min(value + 1, pageCount - 1))
              }
              disabled={currentPage >= pageCount - 1}
              aria-label={t("review.pagination.next")}
            >
              <ChevronRight className="size-4" aria-hidden="true" />
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

function QueueMetric({
  label,
  value,
  tone,
  filterId,
  active,
  onFilter,
}: {
  label: string;
  value: number | string;
  tone: ReviewTone;
  filterId?: string;
  active?: boolean;
  onFilter?: (filterId: string) => void;
}) {
  const { t } = useTranslation("journals");
  const formattedValue =
    typeof value === "number" ? value.toLocaleString("en-US") : value;
  const className = cn(
    "min-w-0 space-y-2 p-3 text-left sm:p-4",
    filterId &&
      "relative w-full cursor-pointer transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
    active && "bg-primary/5 ring-1 ring-primary/30 ring-inset",
  );
  const content = (
    <>
      <p className="text-xs font-medium text-muted-foreground">
        {label}
      </p>
      <p className={cn("text-xl font-semibold tabular-nums", toneTextStyles[tone])}>
        {formattedValue}
      </p>
    </>
  );
  if (!filterId || !onFilter) {
    return <div className={className}>{content}</div>;
  }
  return (
    <button
      type="button"
      className={className}
      onClick={() => onFilter(filterId)}
      aria-pressed={active}
      aria-label={
        filterId === "all"
          ? t("review.metricAria.showAll", { label })
          : t("review.metricAria.filter", { label })
      }
    >
      {content}
    </button>
  );
}

function QuarantineReviewList({
  rows,
  emptyMessage,
  hideSensitive,
  showStateColumn,
  showPriorityBadge,
  sortDirection,
  onSortDirectionChange,
  onOpenTransactionAction,
}: {
  rows: ReviewTableRow[];
  emptyMessage: string;
  hideSensitive: boolean;
  showStateColumn: boolean;
  showPriorityBadge: boolean;
  sortDirection: SortDirection;
  onSortDirectionChange: () => void;
  onOpenTransactionAction?: (
    action: ReviewTransactionAction,
    row: ReviewTableRow,
  ) => void;
}) {
  const { t } = useTranslation(["journals", "common"]);

  if (!rows.length) {
    return (
      <div className="px-3 py-8 sm:px-4">
        <div className="rounded-md border border-dashed border-muted-foreground/40 px-4 py-6 text-center text-sm text-muted-foreground">
          {emptyMessage}
        </div>
      </div>
    );
  }

  return (
    <div className="overflow-x-auto">
      <div className="min-w-[1060px]">
        <div className="flex items-center justify-end border-b bg-muted/30 px-4 py-1.5">
          <SortButton
            label={t("common:field.date")}
            direction={sortDirection}
            onClick={onSortDirectionChange}
          />
        </div>
        <div className="divide-y">
          {rows.map((row) => (
            <QuarantineReviewRow
              key={reviewRowKey(row)}
              row={row}
              hideSensitive={hideSensitive}
              showStateColumn={showStateColumn}
              showPriorityBadge={showPriorityBadge}
              onOpenTransactionAction={onOpenTransactionAction}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function QuarantineReviewRow({
  row,
  hideSensitive,
  showStateColumn,
  showPriorityBadge,
  onOpenTransactionAction,
}: {
  row: ReviewTableRow;
  hideSensitive: boolean;
  showStateColumn: boolean;
  showPriorityBadge: boolean;
  onOpenTransactionAction?: (
    action: ReviewTransactionAction,
    row: ReviewTableRow,
  ) => void;
}) {
  const { t } = useTranslation("journals");
  const StatusIcon = statusIcon[row.status];
  const transactionAction = row.transactionAction;

  return (
    <div
      className={cn(
        "grid grid-cols-[minmax(360px,1.35fr)_minmax(300px,1fr)_minmax(190px,0.55fr)_minmax(210px,0.6fr)] items-stretch gap-0 px-4 py-3 transition-colors hover:bg-muted/35",
        row.status === "Blocked" && "bg-red-500/[0.035] dark:bg-red-950/10",
      )}
    >
      <div className="flex min-w-0 items-start gap-3 pr-4">
        <span
          className={cn(
            "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
            toneStyles[statusTone[row.status]],
          )}
          aria-hidden="true"
        >
          <StatusIcon className="size-4" />
        </span>
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold leading-5">
              {row.event}
            </span>
            {showPriorityBadge ? (
              <Badge
                variant="secondary"
                className={cn("h-5 rounded-md px-1.5 text-[10px]", priorityClass[row.priority])}
              >
                {t(priorityLabelKey[row.priority])}
              </Badge>
            ) : null}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
            <span className="font-mono">{row.id}</span>
            <span aria-hidden="true">·</span>
            <span className={cn("max-w-[16rem] truncate", blurClass(hideSensitive))}>
              {row.account}
            </span>
            <span aria-hidden="true">·</span>
            <span className="font-mono">{row.date}</span>
          </div>
          <p className="mt-2 line-clamp-2 text-xs text-muted-foreground">
            {nextActionLabel(row, t)}
          </p>
        </div>
      </div>

      <div className="min-w-0 border-l px-4">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className={cn("size-2 shrink-0 rounded-full", statusDotClass(row.status))}
            aria-hidden="true"
          />
          <span className="min-w-0 truncate text-xs font-medium text-foreground">
            {row.source}
          </span>
        </div>
        <p className="mt-1.5 line-clamp-2 text-xs leading-5 text-muted-foreground">
          {evidenceHint(row, t)}
        </p>
      </div>

      <div className="min-w-0 border-l px-4 text-right">
        <div
          className={cn(
            "truncate text-sm font-semibold tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {row.amount}
        </div>
        <div
          className={cn(
            "mt-1 truncate font-mono text-[11px] text-muted-foreground",
            blurClass(hideSensitive),
          )}
        >
          {row.basis}
        </div>
        <div
          className={cn(
            "mt-2 truncate text-[11px] font-medium",
            impactToneClass(row.impact),
            blurClass(hideSensitive),
          )}
        >
          {row.impact}
        </div>
      </div>

      <div className="flex min-w-0 flex-col items-end justify-between gap-2 border-l pl-4 text-right">
        {showStateColumn ? (
          <Badge
            variant="outline"
            className={cn("rounded-md", statusClass[row.status])}
          >
            {t(statusLabelKey[row.status])}
          </Badge>
        ) : null}
        {transactionAction ? (
          <ReviewTransactionButton
            transactionAction={transactionAction}
            row={row}
            onOpenTransactionAction={onOpenTransactionAction}
          />
        ) : null}
      </div>
    </div>
  );
}

function ReviewWorklistRow({
  row,
  hideSensitive,
  showStateColumn,
  showPriorityBadge,
  onOpenTransactionAction,
}: {
  row: ReviewTableRow;
  hideSensitive: boolean;
  showStateColumn: boolean;
  showPriorityBadge: boolean;
  onOpenTransactionAction?: (
    action: ReviewTransactionAction,
    row: ReviewTableRow,
  ) => void;
}) {
  const { t } = useTranslation("journals");
  const StatusIcon = statusIcon[row.status];
  const transactionAction = row.transactionAction;

  return (
    <TableRow className="align-top hover:bg-muted/35">
      <TableCell>
        <div className="flex min-w-0 items-start gap-3">
          <span
            className={cn(
              "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
              toneStyles[statusTone[row.status]],
            )}
            aria-hidden="true"
          >
            <StatusIcon className="size-4" />
          </span>
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="truncate text-sm font-medium">{row.event}</span>
              {showPriorityBadge ? (
                <Badge
                  variant="secondary"
                  className={cn("rounded-md", priorityClass[row.priority])}
                >
                  {t(priorityLabelKey[row.priority])}
                </Badge>
              ) : null}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
              <span className="font-mono">{row.id}</span>
              <span aria-hidden="true">·</span>
              <span className={cn("truncate", blurClass(hideSensitive))}>
                {row.account}
              </span>
            </div>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <span className="text-sm text-muted-foreground">{row.source}</span>
        <p className="mt-1 text-[10px] text-muted-foreground sm:text-xs">
          {evidenceHint(row, t)}
        </p>
      </TableCell>
      <TableCell className="text-right">
        <span className={cn("text-sm font-medium tabular-nums", blurClass(hideSensitive))}>
          {row.amount}
        </span>
        <p
          className={cn(
            "mt-1 font-mono text-[10px] text-muted-foreground sm:text-xs",
            blurClass(hideSensitive),
          )}
        >
          {row.basis}
        </p>
      </TableCell>
      <TableCell className="text-right">
        <span
          className={cn(
            "text-sm font-medium tabular-nums",
            impactToneClass(row.impact),
            blurClass(hideSensitive),
          )}
        >
          {row.impact}
        </span>
      </TableCell>
      {showStateColumn ? (
        <TableCell>
          <Badge
            variant="outline"
            className={cn("rounded-md", statusClass[row.status])}
          >
            {t(statusLabelKey[row.status])}
          </Badge>
          <p className="mt-1 text-[10px] text-muted-foreground sm:text-xs">
            {nextActionLabel(row, t)}
          </p>
          {transactionAction ? (
            <ReviewTransactionButton
              transactionAction={transactionAction}
              row={row}
              className="mt-2"
              onOpenTransactionAction={onOpenTransactionAction}
            />
          ) : null}
        </TableCell>
      ) : null}
      <TableCell className="text-right">
        <span className="font-mono text-xs text-muted-foreground">
          {row.date}
        </span>
      </TableCell>
    </TableRow>
  );
}

function ReviewTransactionButton({
  transactionAction,
  row,
  className,
  onOpenTransactionAction,
}: {
  transactionAction: ReviewTransactionAction;
  row: ReviewTableRow;
  className?: string;
  onOpenTransactionAction?: (
    action: ReviewTransactionAction,
    row: ReviewTableRow,
  ) => void;
}) {
  const buttonClassName = cn("h-7 gap-1.5 px-2 text-xs", className);
  if (onOpenTransactionAction) {
    return (
      <Button
        type="button"
        variant="outline"
        size="sm"
        className={buttonClassName}
        onClick={() => onOpenTransactionAction(transactionAction, row)}
      >
        {transactionAction.label}
        <ArrowRight className="size-3.5" aria-hidden="true" />
      </Button>
    );
  }
  return (
    <Button asChild variant="outline" size="sm" className={buttonClassName}>
      <Link
        to="/transactions"
        search={{
          tx: transactionAction.transactionId,
          ...(transactionAction.tab && transactionAction.tab !== "details"
            ? { tab: transactionAction.tab }
            : {}),
        }}
      >
        {transactionAction.label}
        <ArrowRight className="size-3.5" aria-hidden="true" />
      </Link>
    </Button>
  );
}

function SortButton({
  label,
  direction,
  onClick,
}: {
  label: string;
  direction: SortDirection;
  onClick: () => void;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="ml-auto h-8 gap-2 px-2"
      onClick={onClick}
    >
      {label}
      <ArrowDownUp
        className={cn(
          "size-3.5 transition-transform",
          direction === "asc" && "rotate-180",
        )}
        aria-hidden="true"
      />
    </Button>
  );
}

function evidenceHint(row: ReviewTableRow, t: TFunction<"journals">) {
  if (row.evidenceHint) return row.evidenceHint;
  const normalized = `${row.event} ${row.source} ${row.basis}`.toLowerCase();
  if (normalized.includes("price")) return t("review.evidenceHint.price");
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("review.evidenceHint.pair");
  }
  if (normalized.includes("receipt") || normalized.includes("document")) {
    return t("review.evidenceHint.document");
  }
  if (normalized.includes("descriptor") || normalized.includes("asset")) {
    return t("review.evidenceHint.asset");
  }
  if (normalized.includes("fee")) return t("review.evidenceHint.fee");
  return t("review.evidenceHint.fallback");
}

function nextActionLabel(row: ReviewTableRow, t: TFunction<"journals">) {
  if (row.nextAction) return row.nextAction;
  if (row.status === "Resolved") return t("review.nextAction.noAction");
  if (row.status === "Blocked") return t("review.nextAction.blocked");
  if (row.status === "Needs review") return t("review.nextAction.needsReview");
  return t("review.nextAction.ready");
}

function impactToneClass(impact: string) {
  const trimmed = impact.trim();
  if (/^-/.test(trimmed)) return "text-red-600 dark:text-red-400";
  if (/^[+]?[\d€$]/.test(trimmed)) {
    return "text-emerald-600 dark:text-emerald-400";
  }
  return "text-muted-foreground";
}

function statusDotClass(status: ReviewTableRow["status"]) {
  if (status === "Ready" || status === "Resolved") return "bg-emerald-500";
  if (status === "Needs review") return "bg-amber-500";
  return "bg-red-500";
}

const toneStyles: Record<ReviewTone, string> = {
  good:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

const toneBadgeStyles: Record<ReviewTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};

const toneTextStyles: Record<ReviewTone, string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  alert: "text-red-600 dark:text-red-400",
  neutral: "text-foreground",
};
