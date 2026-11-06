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

export interface SettingsSelectOption<T extends string> {
  value: T;
  label: React.ReactNode;
  /** Shown on hover over the option. */
  title?: string;
  disabled?: boolean;
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
  placeholder,
  triggerProps,
}: {
  value: T;
  onValueChange: (value: T) => void;
  options: readonly SettingsSelectOption<T>[];
  disabled?: boolean;
  /** Shown while the value matches no option. */
  placeholder?: string;
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
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem
            key={option.value}
            value={option.value}
            title={option.title}
            disabled={option.disabled}
          >
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * A collapsed list for rarely changed settings: a SettingsGroup's box whose
 * rows show once opened. The native `<details>` keeps it keyboard- and
 * find-in-page friendly; only the marker is replaced, because WebKit and
 * Chromium each draw their own triangle.
 */
export function SettingsDisclosure({
  title,
  open,
  className,
  children,
}: {
  title: React.ReactNode;
  /** Starts open; the user can still fold it. */
  open?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <details
      open={open || undefined}
      className={cn("group overflow-hidden rounded-lg border bg-background", className)}
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 text-sm font-medium select-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none focus-visible:ring-inset [&::-webkit-details-marker]:hidden">
        <ChevronRight
          className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"
          aria-hidden="true"
        />
        {title}
      </summary>
      <div className="divide-y border-t">{children}</div>
    </details>
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

/**
 * A value to copy rather than edit, such as a path or a command: its name,
 * the value in monospace under it, and a copy button. A row of a SettingsGroup.
 */
export function SettingsCopyRow({
  label,
  value,
  copyLabel,
  placeholder,
}: {
  label: React.ReactNode;
  value: string | null;
  copyLabel: string;
  /** Shown while the value is still unknown. */
  placeholder?: string;
}) {
  return (
    <SettingsRow
      label={label}
      description={
        <span className="font-mono text-xs break-all">{value ?? placeholder}</span>
      }
    >
      {value ? <CopyButton value={value} label={copyLabel} /> : null}
    </SettingsRow>
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
