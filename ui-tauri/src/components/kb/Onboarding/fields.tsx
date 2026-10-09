import { useState } from "react";
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

export const ChoiceCard = ({
  active,
  title,
  description,
  detail,
  icon: Icon,
  onClick,
  tone = "default",
}: {
  active: boolean;
  title: string;
  description: string;
  /** The option's trade-off, revealed once it is the chosen one. */
  detail?: string;
  icon?: LucideIcon;
  onClick: () => void;
  tone?: "default" | "warning";
}) => {
  const warning = tone === "warning";
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "group flex w-full cursor-pointer items-start gap-3.5 rounded-(--kb-radius-card) border p-4 text-left text-sm transition-[background-color,border-color,box-shadow] duration-200",
        active
          ? warning
            ? "border-[var(--kb-accent)] bg-[var(--kb-accent)]/[0.04] ring-[3px] ring-[var(--kb-accent)]/10"
            : "border-ink bg-card ring-[3px] ring-ink/[0.07]"
          : "border-border bg-card/60 hover:border-ink/25 hover:bg-card",
      )}
    >
      {Icon && (
        <span
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-(--kb-radius-inset) transition-colors",
            active
              ? warning
                ? "bg-[var(--kb-accent)] text-white"
                : "bg-ink text-paper"
              : "bg-paper-2 text-ink-2 ring-1 ring-border",
          )}
        >
          <Icon className="size-4" aria-hidden="true" />
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="block font-semibold text-ink">{title}</span>
        <span className="mt-1 block text-xs leading-5 text-ink-2">
          {description}
        </span>
        {active && detail && (
          <span className="mt-2.5 block border-t border-border pt-2.5 text-xs leading-5 text-ink-2 animate-in fade-in-0 duration-200">
            {detail}
          </span>
        )}
      </span>
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 flex size-[1.125rem] shrink-0 items-center justify-center rounded-full border transition-colors",
          active
            ? warning
              ? "border-[var(--kb-accent)] bg-[var(--kb-accent)]"
              : "border-ink bg-ink"
            : "border-line-2 group-hover:border-ink/40",
        )}
      >
        {active && <span className="size-1.5 rounded-full bg-paper" />}
      </span>
    </button>
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
  description: string;
}) => {
  return (
    <div
      className={cn(
        "flex items-start gap-3 rounded-(--kb-radius-card) border border-border bg-card/60 p-3.5 transition-colors",
        checked && !disabled && "border-ink/30 bg-card",
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
        <Label htmlFor={id} className="font-semibold text-ink">
          {label}
        </Label>
        <p className="m-0 text-xs leading-5 text-ink-2">{description}</p>
      </div>
    </div>
  );
};
