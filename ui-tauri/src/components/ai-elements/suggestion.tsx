import * as React from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function Suggestions({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="suggestions"
      className={cn("flex flex-wrap justify-center gap-2", className)}
      {...props}
    />
  );
}

function Suggestion({
  className,
  suggestion,
  onClick,
  children,
  ...props
}: Omit<React.ComponentProps<typeof Button>, "onClick"> & {
  suggestion: string;
  onClick?: (suggestion: string) => void;
}) {
  return (
    <Button
      data-slot="suggestion"
      variant="ghost"
      className={cn(
        "group flex h-auto items-center gap-2 rounded-full border border-border/70 bg-background/85 px-3 py-2 text-sm text-foreground shadow-sm transition-colors duration-200 ease-out hover:bg-background dark:bg-muted/80 dark:hover:bg-muted",
        className,
      )}
      onClick={() => onClick?.(suggestion)}
      {...props}
    >
      {children ?? suggestion}
    </Button>
  );
}

export { Suggestion, Suggestions };
