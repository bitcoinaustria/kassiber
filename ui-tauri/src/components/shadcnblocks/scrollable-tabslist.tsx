import * as React from "react"

import { cn } from "@/lib/utils"

function ScrollableTabsList({
  className,
  children,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="scrollable-tabs-list"
      className={cn(
        "min-w-0 overflow-x-auto overflow-y-hidden [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
        className
      )}
      {...props}
    >
      {children}
    </div>
  )
}

export { ScrollableTabsList }
