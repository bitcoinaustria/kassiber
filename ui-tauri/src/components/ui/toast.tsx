import * as React from "react";
import { Toast as ToastPrimitive } from "radix-ui";

import { cn } from "@/lib/utils";

/**
 * Thin shadcn-style wrappers over Radix Toast. Radix supplies the behavior
 * this layer relies on: expiry pauses while the pointer or focus is in the
 * viewport, Escape and swipe dismiss, the viewport hotkey (F8) moves focus
 * there, and each toast is announced through a live region.
 */
function ToastProvider(
  props: React.ComponentProps<typeof ToastPrimitive.Provider>,
) {
  return <ToastPrimitive.Provider data-slot="toast-provider" {...props} />;
}

function ToastViewport({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Viewport>) {
  return (
    <ToastPrimitive.Viewport
      data-slot="toast-viewport"
      className={cn(
        "fixed right-4 bottom-4 z-[100] m-0 flex w-80 max-w-[calc(100vw-2rem)] list-none flex-col gap-2 p-0 outline-none",
        className,
      )}
      {...props}
    />
  );
}

function Toast({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Root>) {
  return (
    <ToastPrimitive.Root
      data-slot="toast"
      className={cn(
        "kb-glass-popover flex items-start gap-2 rounded-md border bg-popover p-3 text-popover-foreground shadow-md data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[swipe=end]:translate-x-(--radix-toast-swipe-end-x) data-[swipe=move]:translate-x-(--radix-toast-swipe-move-x)",
        className,
      )}
      {...props}
    />
  );
}

function ToastTitle({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Title>) {
  return (
    <ToastPrimitive.Title
      data-slot="toast-title"
      className={cn("text-sm font-medium break-words", className)}
      {...props}
    />
  );
}

function ToastDescription({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Description>) {
  return (
    <ToastPrimitive.Description
      data-slot="toast-description"
      className={cn("mt-0.5 text-xs break-words text-muted-foreground", className)}
      {...props}
    />
  );
}

function ToastClose(props: React.ComponentProps<typeof ToastPrimitive.Close>) {
  return <ToastPrimitive.Close data-slot="toast-close" {...props} />;
}

export {
  Toast,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
};
