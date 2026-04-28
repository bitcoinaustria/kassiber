import * as React from "react";
import { Cloud, Cpu, ShieldCheck } from "lucide-react";

import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

type ModelSelectorProviderKind = "local" | "remote" | "tee";

const KIND_STYLES: Record<
  ModelSelectorProviderKind,
  { icon: React.ReactNode; label: string; tone: string }
> = {
  local: {
    icon: <Cpu className="h-3.5 w-3.5" aria-hidden="true" />,
    label: "local",
    tone: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  },
  remote: {
    icon: <Cloud className="h-3.5 w-3.5" aria-hidden="true" />,
    label: "remote",
    tone: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  },
  tee: {
    icon: <ShieldCheck className="h-3.5 w-3.5" aria-hidden="true" />,
    label: "TEE",
    tone: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
  },
};

function ModelSelector({
  ...props
}: React.ComponentProps<typeof Select>) {
  return <Select data-slot="model-selector" {...props} />;
}

function ModelSelectorTrigger({
  className,
  ...props
}: React.ComponentProps<typeof SelectTrigger>) {
  return (
    <SelectTrigger
      data-slot="model-selector-trigger"
      className={cn(
        "h-auto! min-h-0 w-fit border-none bg-transparent! p-0 text-sm leading-none text-muted-foreground shadow-none hover:text-foreground focus:ring-0 focus-visible:border-transparent focus-visible:ring-0",
        className,
      )}
      {...props}
    />
  );
}

function ModelSelectorValue({
  className,
  children,
  ...props
}: React.ComponentProps<typeof SelectValue> & {
  className?: string;
}) {
  return (
    <SelectValue data-slot="model-selector-value" {...props}>
      <span className={cn("truncate", className)}>{children}</span>
    </SelectValue>
  );
}

function ModelSelectorContent({
  className,
  ...props
}: React.ComponentProps<typeof SelectContent>) {
  return (
    <SelectContent
      data-slot="model-selector-content"
      position="popper"
      side="top"
      align="start"
      className={cn("min-w-72", className)}
      {...props}
    />
  );
}

function ModelSelectorGroup({
  ...props
}: React.ComponentProps<typeof SelectGroup>) {
  return <SelectGroup data-slot="model-selector-group" {...props} />;
}

function ModelSelectorLabel({
  className,
  provider,
  kind,
  ...props
}: React.ComponentProps<typeof SelectLabel> & {
  provider?: string;
  kind?: ModelSelectorProviderKind;
}) {
  const kindStyle = kind ? KIND_STYLES[kind] : null;

  return (
    <SelectLabel
      data-slot="model-selector-label"
      className={cn("flex items-center gap-2", className)}
      {...props}
    >
      {provider ? <span>{provider}</span> : props.children}
      {kindStyle ? (
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] font-medium uppercase",
            kindStyle.tone,
          )}
        >
          {kindStyle.icon}
          {kindStyle.label}
        </span>
      ) : null}
    </SelectLabel>
  );
}

function ModelSelectorItem({
  className,
  children,
  ...props
}: React.ComponentProps<typeof SelectItem>) {
  return (
    <SelectItem
      data-slot="model-selector-item"
      className={cn("font-mono text-xs", className)}
      {...props}
    >
      {children}
    </SelectItem>
  );
}

function ModelSelectorName({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="model-selector-name"
      className={cn("truncate", className)}
      {...props}
    />
  );
}

function ModelSelectorEmpty({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="model-selector-empty"
      className={cn("text-muted-foreground", className)}
      {...props}
    />
  );
}

export {
  ModelSelector,
  ModelSelectorContent,
  ModelSelectorEmpty,
  ModelSelectorGroup,
  ModelSelectorItem,
  ModelSelectorLabel,
  ModelSelectorName,
  ModelSelectorTrigger,
  ModelSelectorValue,
  type ModelSelectorProviderKind,
};
