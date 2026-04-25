import Ai02 from "@/components/ai-02";
import { cn } from "@/lib/utils";
import * as React from "react";

interface ScreenAssistantMockupProps {
  className?: string;
  collapsed?: boolean;
}

export function ScreenAssistantMockup({
  className,
  collapsed = false,
}: ScreenAssistantMockupProps) {
  const [isInteracting, setIsInteracting] = React.useState(false);
  const compact = collapsed && !isInteracting;

  return (
    <section
      aria-label="Kassiber assistant"
      className={cn("pointer-events-none px-3 pb-4 sm:px-4 md:px-6", className)}
    >
      <div
        className="pointer-events-auto"
        onMouseEnter={() => setIsInteracting(true)}
        onMouseLeave={() => setIsInteracting(false)}
        onFocusCapture={() => setIsInteracting(true)}
        onBlurCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setIsInteracting(false);
          }
        }}
      >
        <Ai02 compact={compact} />
      </div>
    </section>
  );
}
