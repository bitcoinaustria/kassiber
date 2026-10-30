import * as React from "react";
import { Check, ChevronRight, Copy } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { copyTextWithPolicy } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

export function CopyButton({
  value,
  label,
}: {
  value: string;
  label: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const onCopy = async () => {
    try {
      await copyTextWithPolicy(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard can be unavailable (e.g. browser permissions); ignore.
    }
  };
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="ghost"
      aria-label={label}
      onClick={() => void onCopy()}
    >
      {copied ? (
        <Check className="size-3.5 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
      ) : (
        <Copy className="size-3.5" aria-hidden="true" />
      )}
    </Button>
  );
}

/**
 * A settings list in the manner of the ChatGPT desktop app's settings: one
 * hairline box, rows divided inside it, each row the setting's name and a line
 * of explanation on the left and its control on the right. Actions for the
 * whole list sit beside the title; a note about all of it goes under the box.
 */
export function SettingsGroup({
  title,
  actions,
  footer,
  className,
  children,
  ...rest
}: {
  title?: React.ReactNode;
  actions?: React.ReactNode;
  footer?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
} & Omit<React.HTMLAttributes<HTMLElement>, "title" | "children">) {
  return (
    <section className={cn("space-y-2", className)} {...rest}>
      {title || actions ? (
        <div className="flex items-end justify-between gap-3">
          {title ? <h3 className="text-sm font-semibold">{title}</h3> : <span />}
          {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
        </div>
      ) : null}
      <div className="divide-y overflow-hidden rounded-lg border bg-background">{children}</div>
      {footer ? <p className="px-1 text-xs text-muted-foreground">{footer}</p> : null}
    </section>
  );
}

export function SettingsRow({
  label,
  description,
  htmlFor,
  children,
}: {
  label: React.ReactNode;
  description?: React.ReactNode;
  /** The control's id, so a click on the name focuses or toggles it. */
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
      <div className="min-w-0 space-y-0.5">
        <Label htmlFor={htmlFor} className="text-sm font-medium">
          {label}
        </Label>
        {description ? (
          <p className="text-sm text-muted-foreground">{description}</p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export interface SegmentedOption<T extends string> {
  value: T;
  label: React.ReactNode;
  icon?: React.ReactNode;
}

/**
 * A choice of a few exclusive values, drawn as one pill of segments. A radio
 * group, not tabs: picking a theme changes a setting, it does not switch a
 * panel, so arrows move the choice the way they do in a native radio group.
 */
export function SegmentedControl<T extends string>({
  value,
  onValueChange,
  options,
  label,
}: {
  value: T;
  onValueChange: (value: T) => void;
  options: readonly SegmentedOption<T>[];
  label: string;
}) {
  const refs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const select = (index: number) => {
    const option = options[(index + options.length) % options.length];
    onValueChange(option.value);
    refs.current[(index + options.length) % options.length]?.focus();
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="inline-flex items-center gap-0.5 rounded-lg bg-muted p-0.5"
    >
      {options.map((option, index) => {
        const checked = option.value === value;
        return (
          <button
            key={option.value}
            ref={(node) => {
              refs.current[index] = node;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            onClick={() => onValueChange(option.value)}
            onKeyDown={(event) => {
              if (event.key === "ArrowRight" || event.key === "ArrowDown") {
                event.preventDefault();
                select(index + 1);
              } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
                event.preventDefault();
                select(index - 1);
              }
            }}
            className={cn(
              "inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-sm font-medium whitespace-nowrap transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none [&_svg]:size-4 [&_svg]:shrink-0",
              checked
                ? "bg-background text-foreground shadow-xs dark:bg-input/60"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export interface SettingsSelectOption<T extends string> {
  value: T;
  label: React.ReactNode;
  /** Shown on hover over the option. */
  title?: string;
}

/**
 * A choice among values, as a dropdown of one width and height wherever it
 * sits, so a column of choices lines up. Only listed values are passed on.
 */
export function SettingsSelect<T extends string>({
  value,
  onValueChange,
  options,
  disabled = false,
  triggerProps,
}: {
  value: T;
  onValueChange: (value: T) => void;
  options: readonly SettingsSelectOption<T>[];
  disabled?: boolean;
  /** id, aria-* and data-* for the trigger. */
  triggerProps?: Omit<React.ComponentProps<typeof SelectTrigger>, "size" | "children"> & {
    [data: `data-${string}`]: string | undefined;
  };
}) {
  return (
    <Select
      value={value}
      disabled={disabled}
      onValueChange={(next) => {
        const option = options.find((candidate) => candidate.value === next);
        if (option) onValueChange(option.value);
      }}
    >
      <SelectTrigger
        size="sm"
        {...triggerProps}
        className={cn("w-48", triggerProps?.className)}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value} title={option.title}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * A collapsed section for rarely changed settings. The native `<details>`
 * keeps it keyboard- and find-in-page friendly; only the marker is replaced,
 * because WebKit and Chromium each draw their own triangle.
 */
export function SettingsDisclosure({
  title,
  className,
  children,
}: {
  title: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <details className={cn("group rounded-lg border bg-background", className)}>
      <summary className="flex cursor-pointer list-none items-center gap-2 rounded-lg px-4 py-3 text-sm font-medium select-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none [&::-webkit-details-marker]:hidden">
        <ChevronRight
          className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
          aria-hidden="true"
        />
        {title}
      </summary>
      <div className="space-y-3 border-t px-4 py-3">{children}</div>
    </details>
  );
}

export interface SettingsSwitchRowProps {
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
}

export function SettingsSwitchRow({
  label,
  description,
  checked,
  onCheckedChange,
  disabled = false,
}: SettingsSwitchRowProps) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-4 rounded-lg border bg-background p-3",
        disabled && "opacity-60",
      )}
    >
      <div className="min-w-0 space-y-1">
        <Label className="text-sm font-medium">{label}</Label>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <Switch
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={disabled}
      />
    </div>
  );
}

export function CommandLine({ command }: { command: string }) {
  return (
    <div className="flex items-center gap-2 rounded-md border bg-muted/40 px-3 py-1.5">
      <code className="min-w-0 flex-1 truncate font-mono text-xs">{command}</code>
      <CopyButton value={command} label={`Copy "${command}"`} />
    </div>
  );
}

export function PathField({
  id,
  label,
  value,
}: {
  id: string;
  label: string;
  value: string | null;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <div className="flex items-center gap-1">
        <Input
          id={id}
          readOnly
          value={value ?? "loading…"}
          className="font-mono text-xs"
        />
        {value ? <CopyButton value={value} label={`Copy ${label}`} /> : null}
      </div>
    </div>
  );
}

export interface SecretFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

export function SecretField({
  id,
  label,
  value,
  onChange,
  placeholder,
}: SecretFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}
