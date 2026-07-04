import { useState } from "react";
import { Check } from "lucide-react";

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
          className="m-0 font-mono text-[10px] uppercase tracking-[0.08em] text-[var(--kb-accent)]"
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
          className="m-0 font-mono text-[10px] uppercase tracking-[0.08em] text-[var(--kb-accent)]"
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
  onClick,
  tone = "default",
}: {
  active: boolean;
  title: string;
  description: string;
  onClick: () => void;
  tone?: "default" | "warning";
}) => {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "flex min-h-[84px] cursor-pointer items-start gap-3 rounded-lg border p-3.5 text-left text-sm transition",
        active
          ? tone === "warning"
            ? "border-[var(--kb-accent)] bg-[rgba(227,0,15,0.04)]"
            : "border-ink bg-paper"
          : "border-line hover:bg-paper-2",
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border",
          active ? "border-ink bg-ink text-paper" : "border-line",
        )}
      >
        {active && <Check className="size-3.5" />}
      </span>
      <span>
        <span className="block font-semibold text-ink">{title}</span>
        <span className="mt-1 block text-xs leading-5 text-ink-2">
          {description}
        </span>
      </span>
    </button>
  );
};

export const CheckRow = ({
  id,
  checked,
  onCheckedChange,
  label,
  description,
}: {
  id: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  label: string;
  description: string;
}) => {
  return (
    <div className="flex items-start gap-3 rounded-lg border border-line p-3">
      <Checkbox
        id={id}
        checked={checked}
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
