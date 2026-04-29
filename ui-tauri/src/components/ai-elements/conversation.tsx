import * as React from "react";

import { cn } from "@/lib/utils";

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
      "flex min-h-0 w-full flex-1 flex-col gap-6 overflow-y-auto px-1",
      className,
    )}
    {...props}
  />
));
ConversationContent.displayName = "ConversationContent";

export { Conversation, ConversationContent };
