import { useState, type ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

import { NetworkStatus } from "./network";
import type { NetworkSummary } from "./networkReach";

export const TextField = ({
  label,
  name,
  value,
  placeholder,
  type = "text",
  autoComplete,
  autoFocus,
  hint,
  description,
  disabled,
  onChange,
}: {
  label: string;
  name: string;
  value: string;
  placeholder: string;
  type?: string;
  autoComplete?: string;
  autoFocus?: boolean;
  hint?: string | null;
  description?: string | null;
  disabled?: boolean;
  onChange: (value: string) => void;
}) => {
  // Hold validation hints until the field has been touched or has content, so a
  // pristine step never greets the user with a red error before they type.
  const [touched, setTouched] = useState(false);
  const showHint = Boolean(hint) && (touched || value.length > 0);
  return (
    <div className="space-y-2">
      <Label htmlFor={name}>{label}</Label>
      <Input
        id={name}
        name={name}
        type={type}
        autoComplete={autoComplete}
        autoFocus={autoFocus}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        onBlur={() => setTouched(true)}
        aria-invalid={showHint ? true : undefined}
        aria-describedby={showHint ? `${name}-hint` : undefined}
        className="w-full rounded-md border-line"
      />
      {showHint && (
        <p
          id={`${name}-hint`}
          className="m-0 font-mono text-2xs uppercase tracking-[0.08em] text-[var(--kb-accent)]"
        >
          {hint}
        </p>
      )}
      {!showHint && description && (
        <p className="m-0 text-xs leading-5 text-ink-2">{description}</p>
      )}
    </div>
  );
};

export const NumberField = ({
  label,
  name,
  value,
  placeholder,
  min,
  onChange,
  hint,
  description,
}: {
  label: string;
  name: string;
  value: string;
  placeholder: string;
  min?: number;
  onChange: (value: string) => void;
  hint?: string | null;
  description?: string | null;
}) => {
  const [touched, setTouched] = useState(false);
  const showHint = Boolean(hint) && (touched || value.length > 0);
  return (
    <div className="space-y-2">
      <Label htmlFor={name}>{label}</Label>
      <Input
        id={name}
        name={name}
        type="number"
        inputMode="numeric"
        min={min}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        onBlur={() => setTouched(true)}
        aria-invalid={showHint ? true : undefined}
        aria-describedby={showHint ? `${name}-hint` : undefined}
        className="w-full rounded-md border-line"
      />
      {showHint && (
        <p
          id={`${name}-hint`}
          className="m-0 font-mono text-2xs uppercase tracking-[0.08em] text-[var(--kb-accent)]"
        >
          {hint}
        </p>
      )}
      {!showHint && description && (
        <p className="m-0 text-xs leading-5 text-ink-2">{description}</p>
      )}
    </div>
  );
};

export const SelectField = <T extends string>({
  label,
  value,
  options,
  optionLabels,
  description,
  onChange,
}: {
  label: string;
  value: T;
  options: T[];
  optionLabels?: Partial<Record<T, string>>;
  description?: string | null;
  onChange: (value: T) => void;
}) => {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <Select value={value} onValueChange={(next) => onChange(next as T)}>
        <SelectTrigger className="w-full rounded-md border-line">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {optionLabels?.[option] ?? option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {description && (
        <p className="m-0 text-xs leading-5 text-ink-2">{description}</p>
      )}
    </div>
  );
};

/**
 * One option of a step's main decision, after T3 Code's welcome-wizard option
 * cards (MIT, Copyright (c) 2026 T3 Tools Inc.): a radio on the left, the
 * title with what the option reaches on the network on the right, and the
 * hosts it would contact underneath. The chosen card holds what that choice
 * needs (`children`).
 */
export const ChoiceCard = ({
  active,
  title,
  description,
  detail,
  network,
  icon: Icon,
  onClick,
  tone = "default",
  children,
}: {
  active: boolean;
  title: string;
  description: string;
  /** The option's trade-off, shown on every card so options compare. */
  detail?: string;
  /** What choosing this lets Kassiber contact, and when. */
  network?: NetworkSummary;
  icon?: LucideIcon;
  onClick: () => void;
  tone?: "default" | "warning";
  /** Shown inside the card while it is the chosen one. */
  children?: ReactNode;
}) => {
  const warning = tone === "warning";
  return (
    <div
      className={cn(
        "rounded-(--kb-radius-card) border transition-colors duration-150",
        active
          ? warning
            ? "border-[var(--kb-accent)]/70 bg-card ring-1 ring-[var(--kb-accent)]/15"
            : "border-ink/40 bg-card ring-1 ring-ink/10"
          : "border-border bg-background hover:border-ink/25",
      )}
    >
      <button
        type="button"
        aria-pressed={active}
        onClick={onClick}
        className="group flex w-full cursor-pointer items-start gap-3 rounded-(--kb-radius-card) px-3.5 py-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span
          aria-hidden="true"
          className={cn(
            "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border transition-colors",
            active
              ? warning
                ? "border-[var(--kb-accent)] bg-[var(--kb-accent)]"
                : "border-primary bg-primary"
              : "border-line-2 group-hover:border-ink/40",
          )}
        >
          {active && <span className="size-1.5 rounded-full bg-paper" />}
        </span>
        {Icon && (
          <Icon
            className={cn(
              "mt-0.5 size-4 shrink-0",
              active ? "text-ink" : "text-ink-3",
            )}
            aria-hidden="true"
          />
        )}
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
            <span className="text-sm font-medium text-ink">{title}</span>
            {network && (
              <NetworkStatus reach={network.reach} label={network.label} />
            )}
          </span>
          <span className="mt-1 block text-xs leading-normal text-ink-3">
            {description}
          </span>
          {network?.hosts?.length ? (
            // One server per line. Block spans, not a list: this sits
            // inside the option's button.
            <span className="mt-1.5 block space-y-0.5">
              {network.hosts.map(({ host, label }) => (
                <span
                  key={host}
                  className="grid gap-x-4 text-2xs leading-5 sm:grid-cols-[minmax(0,15rem)_1fr]"
                >
                  <span className="font-mono break-all text-ink-2">{host}</span>
                  {label && <span className="text-ink-3">{label}</span>}
                </span>
              ))}
            </span>
          ) : network?.pending ? (
            <span className="mt-1.5 block text-xs text-ink-3 italic">
              {network.pending}
            </span>
          ) : null}
          {detail && (
            <span
              className={cn(
                "mt-1.5 block text-xs leading-normal",
                active ? "text-ink-2" : "text-ink-3",
              )}
            >
              {detail}
            </span>
          )}
        </span>
      </button>
      {active && children && (
        <div
          className={cn(
            "space-y-4 border-t border-border/60 px-3.5 py-4 animate-in fade-in-0 duration-150",
            Icon ? "sm:pl-[3.75rem]" : "sm:pl-[2.25rem]",
          )}
        >
          {children}
        </div>
      )}
    </div>
  );
};

export const CheckRow = ({
  id,
  checked,
  disabled = false,
  onCheckedChange,
  label,
  description,
}: {
  id: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
  description: ReactNode;
}) => {
  return (
    <div
      className={cn(
        "flex items-start gap-3 transition-opacity",
        disabled && "opacity-60",
      )}
    >
      <Checkbox
        id={id}
        checked={checked}
        disabled={disabled}
        onCheckedChange={(value) => onCheckedChange(value === true)}
        className="mt-0.5"
      />
      <div className="grid gap-1">
        <Label htmlFor={id} className="font-medium text-ink">
          {label}
        </Label>
        <div className="text-xs leading-5 text-ink-2">{description}</div>
      </div>
    </div>
  );
};
