import {
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  useReactTable,
  type ColumnDef,
  type ColumnFiltersState,
  type SortingState,
} from "@tanstack/react-table";
import {
  ArrowDownUp,
  ChevronLeft,
  ChevronRight,
  Search,
} from "lucide-react";
import { useMemo, useState, type ComponentType, type SVGProps } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

export type ReviewTableKind = "tax-events" | "quarantine";

export interface ReviewTableRow {
  id: string;
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
}

interface ReviewDataTableProps {
  kind: ReviewTableKind;
  eyebrow: string;
  title: string;
  description: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
  rows: ReviewTableRow[];
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

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function ReviewDataTable({
  kind,
  eyebrow,
  title,
  description,
  icon: Icon,
  rows,
}: ReviewDataTableProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [sorting, setSorting] = useState<SortingState>([
    { id: "date", desc: true },
  ]);
  const [columnFilters, setColumnFilters] = useState<ColumnFiltersState>([]);
  const [globalFilter, setGlobalFilter] = useState("");
  const [rowSelection, setRowSelection] = useState({});

  const columns = useMemo<ColumnDef<ReviewTableRow>[]>(
    () => [
      {
        id: "select",
        header: ({ table }) => (
          <Checkbox
            checked={
              table.getIsAllPageRowsSelected() ||
              (table.getIsSomePageRowsSelected() && "indeterminate")
            }
            onCheckedChange={(value) =>
              table.toggleAllPageRowsSelected(!!value)
            }
            aria-label="Select all rows"
          />
        ),
        cell: ({ row }) => (
          <Checkbox
            checked={row.getIsSelected()}
            onCheckedChange={(value) => row.toggleSelected(!!value)}
            aria-label={`Select ${row.original.id}`}
          />
        ),
        enableSorting: false,
        enableHiding: false,
      },
      {
        accessorKey: "id",
        header: "Record",
        cell: ({ row }) => (
          <div className="font-mono text-xs text-muted-foreground">
            {row.original.id}
          </div>
        ),
      },
      {
        accessorKey: "date",
        header: ({ column }) => (
          <SortButton
            label="Date"
            onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
          />
        ),
        cell: ({ row }) => (
          <span className="font-mono text-xs text-muted-foreground">
            {row.original.date}
          </span>
        ),
      },
      {
        accessorKey: "event",
        header: kind === "tax-events" ? "Tax event" : "Issue",
        cell: ({ row }) => (
          <div className="min-w-44">
            <p className="font-medium">{row.original.event}</p>
            <p className="text-xs text-muted-foreground">
              {row.original.account}
            </p>
          </div>
        ),
      },
      {
        accessorKey: "source",
        header: "Source",
        cell: ({ row }) => (
          <span className="text-muted-foreground">{row.original.source}</span>
        ),
      },
      {
        accessorKey: "amount",
        header: "Amount",
        cell: ({ row }) => (
          <span className={cn("font-medium", blurClass(hideSensitive))}>
            {row.original.amount}
          </span>
        ),
      },
      {
        accessorKey: "basis",
        header: "Basis",
        cell: ({ row }) => (
          <span
            className={cn(
              "font-mono text-xs text-muted-foreground",
              blurClass(hideSensitive),
            )}
          >
            {row.original.basis}
          </span>
        ),
      },
      {
        accessorKey: "impact",
        header: "Impact",
        cell: ({ row }) => (
          <span
            className={cn(
              "font-medium",
              row.original.impact.startsWith("-")
                ? "text-red-600"
                : "text-emerald-600",
              blurClass(hideSensitive),
            )}
          >
            {row.original.impact}
          </span>
        ),
      },
      {
        accessorKey: "status",
        header: "Status",
        cell: ({ row }) => (
          <Badge
            variant="outline"
            className={cn("rounded-md", statusClass[row.original.status])}
          >
            {row.original.status}
          </Badge>
        ),
      },
      {
        accessorKey: "priority",
        header: "Priority",
        cell: ({ row }) => (
          <Badge
            variant="secondary"
            className={cn("rounded-md", priorityClass[row.original.priority])}
          >
            {row.original.priority}
          </Badge>
        ),
      },
      {
        accessorKey: "owner",
        header: "Owner",
        cell: ({ row }) => (
          <span className="text-muted-foreground">{row.original.owner}</span>
        ),
      },
    ],
    [hideSensitive, kind],
  );

  const table = useReactTable({
    data: rows,
    columns,
    state: {
      sorting,
      columnFilters,
      globalFilter,
      rowSelection,
    },
    onSortingChange: setSorting,
    onColumnFiltersChange: setColumnFilters,
    onGlobalFilterChange: setGlobalFilter,
    onRowSelectionChange: setRowSelection,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: {
      pagination: { pageSize: 8 },
    },
  });

  const selectedCount = table.getFilteredSelectedRowModel().rows.length;
  const visibleRows = table.getFilteredRowModel().rows.length;
  const blockedCount = rows.filter((row) => row.status === "Blocked").length;
  const reviewCount = rows.filter((row) => row.status === "Needs review").length;
  const statusFilter = table.getColumn("status")?.getFilterValue();
  const setStatusFilter = (status?: ReviewTableRow["status"]) => {
    table.getColumn("status")?.setFilterValue(status);
  };

  return (
    <div className="w-full space-y-4 bg-background p-3 sm:p-4 md:p-6">
      <div className="grid gap-3 md:grid-cols-3">
        <Card className="md:col-span-2">
          <CardHeader className="flex flex-row items-start justify-between gap-4">
            <div className="min-w-0 space-y-2">
              <p className="text-xs font-medium tracking-[0.2em] text-muted-foreground uppercase">
                {eyebrow}
              </p>
              <CardTitle className="text-3xl">{title}</CardTitle>
              <CardDescription className="max-w-3xl text-sm">
                {description}
              </CardDescription>
            </div>
            <div className="flex size-11 shrink-0 items-center justify-center rounded-md border bg-muted/50">
              <Icon className="size-5" aria-hidden="true" />
            </div>
          </CardHeader>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardDescription>Review queue</CardDescription>
            <CardTitle className="text-3xl">{visibleRows}</CardTitle>
          </CardHeader>
          <CardContent className="grid grid-cols-2 gap-2 text-sm">
            <div className="rounded-md bg-muted/50 p-3">
              <p className="text-xs text-muted-foreground">Needs review</p>
              <p className="text-xl font-semibold">{reviewCount}</p>
            </div>
            <div className="rounded-md bg-muted/50 p-3">
              <p className="text-xs text-muted-foreground">Blocked</p>
              <p className="text-xl font-semibold">{blockedCount}</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="border-b">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <CardTitle>{title}</CardTitle>
              <CardDescription>
                {selectedCount
                  ? `${selectedCount} selected`
                  : `${visibleRows} records in this view`}
              </CardDescription>
            </div>
            <div className="flex flex-col gap-2 sm:flex-row">
              <div className="relative min-w-0 sm:w-80">
                <Search
                  className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
                  aria-hidden="true"
                />
                <Input
                  value={globalFilter}
                  onChange={(event) => setGlobalFilter(event.target.value)}
                  placeholder="Search account, event, source..."
                  className="pl-9"
                />
              </div>
              <Button
                type="button"
                variant={!statusFilter ? "default" : "outline"}
                onClick={() => setStatusFilter()}
              >
                All
              </Button>
              <Button
                type="button"
                variant={statusFilter === "Needs review" ? "default" : "outline"}
                onClick={() => setStatusFilter("Needs review")}
              >
                Needs review
              </Button>
              <Button
                type="button"
                variant={statusFilter === "Blocked" ? "default" : "outline"}
                onClick={() => setStatusFilter("Blocked")}
              >
                Blocked
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              {table.getHeaderGroups().map((headerGroup) => (
                <TableRow
                  key={headerGroup.id}
                  className="bg-muted/40 hover:bg-muted/40"
                >
                  {headerGroup.headers.map((header) => (
                    <TableHead key={header.id}>
                      {header.isPlaceholder
                        ? null
                        : flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                    </TableHead>
                  ))}
                </TableRow>
              ))}
            </TableHeader>
            <TableBody>
              {table.getRowModel().rows.length ? (
                table.getRowModel().rows.map((row) => (
                  <TableRow
                    key={row.id}
                    data-state={row.getIsSelected() && "selected"}
                  >
                    {row.getVisibleCells().map((cell) => (
                      <TableCell key={cell.id}>
                        {flexRender(
                          cell.column.columnDef.cell,
                          cell.getContext(),
                        )}
                      </TableCell>
                    ))}
                  </TableRow>
                ))
              ) : (
                <TableRow>
                  <TableCell
                    colSpan={columns.length}
                    className="h-24 text-center text-muted-foreground"
                  >
                    No matching records.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
          <div className="flex flex-col gap-3 border-t px-4 py-3 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
            <span>
              {selectedCount} of {visibleRows} row(s) selected
            </span>
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="icon"
                className="size-8"
                onClick={() => table.previousPage()}
                disabled={!table.getCanPreviousPage()}
                aria-label="Previous page"
              >
                <ChevronLeft className="size-4" aria-hidden="true" />
              </Button>
              <span>
                Page {table.getState().pagination.pageIndex + 1} of{" "}
                {Math.max(table.getPageCount(), 1)}
              </span>
              <Button
                variant="outline"
                size="icon"
                className="size-8"
                onClick={() => table.nextPage()}
                disabled={!table.getCanNextPage()}
                aria-label="Next page"
              >
                <ChevronRight className="size-4" aria-hidden="true" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function SortButton({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className="-ml-3 h-8 gap-2 px-2"
      onClick={onClick}
    >
      {label}
      <ArrowDownUp className="size-3.5" aria-hidden="true" />
    </Button>
  );
}
