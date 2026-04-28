import * as React from "react";

import { cn } from "@/lib/utils";

function Context({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="context"
      className={cn(
        "flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function ContextItem({
  className,
  icon,
  label,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  icon?: React.ReactNode;
  label?: string;
}) {
  return (
    <div
      data-slot="context-item"
      className={cn(
        "flex min-w-0 items-center gap-1.5 rounded-full bg-muted px-2 py-1",
        className,
      )}
      {...props}
    >
      {icon}
      {label ? <span className="sr-only">{label}</span> : null}
      {children}
    </div>
  );
}

export { Context, ContextItem };
