import * as React from "react";
import { ShieldCheck } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function Confirmation({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="confirmation"
      className={cn(
        "rounded-md border border-border/70 bg-muted/30 px-3 py-2 text-sm",
        className,
      )}
      {...props}
    />
  );
}

function ConfirmationTitle({
  className,
  children,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="confirmation-title"
      className={cn("flex items-center gap-2 font-medium", className)}
      {...props}
    >
      <ShieldCheck className="h-4 w-4 text-primary" aria-hidden="true" />
      <span>{children}</span>
    </div>
  );
}

function ConfirmationRequest({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="confirmation-request"
      className={cn("mt-2 text-muted-foreground", className)}
      {...props}
    />
  );
}

function ConfirmationActions({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="confirmation-actions"
      className={cn("mt-3 flex flex-wrap justify-end gap-2", className)}
      {...props}
    />
  );
}

function ConfirmationAction({
  ...props
}: React.ComponentProps<typeof Button>) {
  return <Button data-slot="confirmation-action" size="sm" {...props} />;
}

export {
  Confirmation,
  ConfirmationAction,
  ConfirmationActions,
  ConfirmationRequest,
  ConfirmationTitle,
};
