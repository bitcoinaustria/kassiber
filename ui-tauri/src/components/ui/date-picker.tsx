import * as React from "react";
import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

// Monday-first week (EU/AT convention).
const WEEKDAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

interface YMD {
  y: number;
  m: number; // 0-based
  d: number;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

// Build the ISO string from y/m/d directly — never via Date.toISOString(), which
// is UTC and would shift the day across a timezone boundary.
function toISO(y: number, m: number, d: number): string {
  return `${y}-${pad(m + 1)}-${pad(d)}`;
}

function parseISO(value: string | undefined): YMD {
  if (value) {
    const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value);
    if (match) {
      return { y: Number(match[1]), m: Number(match[2]) - 1, d: Number(match[3]) };
    }
  }
  const now = new Date();
  return { y: now.getFullYear(), m: now.getMonth(), d: now.getDate() };
}

function formatDisplay(value: string | undefined): string {
  if (!value) return "Pick a date";
  const { y, m, d } = parseISO(value);
  return new Intl.DateTimeFormat(undefined, {
    day: "2-digit",
    month: "short",
    year: "numeric",
  }).format(new Date(y, m, d));
}

interface DatePickerProps {
  /** ISO `yyyy-mm-dd`. */
  value: string;
  onChange: (value: string) => void;
  className?: string;
  id?: string;
}

/**
 * Popover + month-grid calendar date picker built on the app's shadcn/radix-ui
 * primitives (no react-day-picker / date-fns dependency). Value is an ISO
 * `yyyy-mm-dd` string.
 */
export function DatePicker({ value, onChange, className, id }: DatePickerProps) {
  const [open, setOpen] = React.useState(false);
  const selected = parseISO(value);
  const today = parseISO(undefined);
  const [view, setView] = React.useState({ y: selected.y, m: selected.m });

  // Re-center on the selected month whenever the popover opens.
  const handleOpenChange = (next: boolean) => {
    if (next) {
      const sel = parseISO(value);
      setView({ y: sel.y, m: sel.m });
    }
    setOpen(next);
  };

  const firstWeekday = (new Date(view.y, view.m, 1).getDay() + 6) % 7; // Mon=0
  const daysInMonth = new Date(view.y, view.m + 1, 0).getDate();
  const cells: Array<number | null> = [
    ...Array<null>(firstWeekday).fill(null),
    ...Array.from({ length: daysInMonth }, (_, i) => i + 1),
  ];

  const stepMonth = (delta: number) => {
    setView((prev) => {
      const total = prev.y * 12 + prev.m + delta;
      return { y: Math.floor(total / 12), m: ((total % 12) + 12) % 12 };
    });
  };

  const pick = (day: number) => {
    onChange(toISO(view.y, view.m, day));
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <Button
          id={id}
          type="button"
          variant="outline"
          className={cn("h-9 justify-start gap-2 px-3 font-normal", className)}
        >
          <CalendarDays className="size-4 opacity-70" />
          <span className={cn(!value && "text-muted-foreground")}>
            {formatDisplay(value)}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-3" align="start">
        <div className="flex items-center justify-between pb-2">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={() => stepMonth(-1)}
            aria-label="Previous month"
          >
            <ChevronLeft className="size-4" />
          </Button>
          <div className="text-sm font-medium">
            {MONTHS[view.m]} {view.y}
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={() => stepMonth(1)}
            aria-label="Next month"
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>
        <div className="grid grid-cols-7 gap-1 text-center text-[11px] font-medium text-muted-foreground">
          {WEEKDAYS.map((weekday) => (
            <div key={weekday} className="py-1">
              {weekday}
            </div>
          ))}
        </div>
        <div className="grid grid-cols-7 gap-1">
          {cells.map((day, index) => {
            if (day === null) return <div key={`blank-${index}`} />;
            const isSelected =
              day === selected.d && view.m === selected.m && view.y === selected.y;
            const isToday =
              day === today.d && view.m === today.m && view.y === today.y;
            return (
              <button
                key={day}
                type="button"
                onClick={() => pick(day)}
                aria-pressed={isSelected}
                className={cn(
                  "flex size-8 items-center justify-center rounded-md text-sm tabular-nums transition-colors hover:bg-accent hover:text-accent-foreground",
                  isSelected &&
                    "bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground",
                  !isSelected && isToday && "border border-ring",
                )}
              >
                {day}
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
