import * as React from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { ArrowDown } from "lucide-react";

function Conversation({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="conversation"
      className={cn("relative flex min-h-0 w-full flex-1 flex-col", className)}
      {...props}
    />
  );
}

const ConversationContent = React.forwardRef<
  HTMLDivElement,
  React.ComponentProps<"div">
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    data-slot="conversation-content"
    className={cn(
      "flex min-h-0 w-full flex-1 flex-col gap-3 overflow-y-auto px-1",
      className,
    )}
    {...props}
  />
));
ConversationContent.displayName = "ConversationContent";

function ConversationScrollButton({
  className,
  ...props
}: React.ComponentProps<typeof Button>) {
  return (
    <Button
      data-slot="conversation-scroll-button"
      variant="secondary"
      size="icon-sm"
      className={cn(
        "absolute right-3 bottom-3 rounded-full border border-border/70 shadow-sm",
        className,
      )}
      {...props}
    >
      <ArrowDown aria-hidden="true" />
      <span className="sr-only">Scroll to latest message</span>
    </Button>
  );
}

export { Conversation, ConversationContent, ConversationScrollButton };
