import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

/**
 * Card surface.
 *
 * Cards are NOT frosted, deliberately. `backdrop-filter` blurs whatever is
 * behind an element, and a card sits on the flat page background — blurring a
 * flat colour returns that same colour, so glass here would cost a compositing
 * layer per card and render nothing. Depth comes from a hairline edge and a
 * two-layer shadow instead; glass is reserved for surfaces that genuinely float
 * over content (see `.kb-glass-*`).
 *
 * The elevation below was not invented here: it was already hand-rolled in
 * `ConnectionDetail` and `UtxosInventoryPanel` and copy-pasted seven times.
 * Promoting it to the primitive is what removes those copies.
 */
const cardVariants = cva(
  // `.kb-surface` carries the panel tier's radius, hairline edge and two-layer
  // shadow (see globals.css) so the elevation has exactly one definition.
  "kb-surface flex flex-col text-card-foreground",
  {
    variants: {
      variant: {
        default: "gap-6 py-6",
        /**
         * For a card whose child must reach the edges — a table, or a list with
         * its own row dividers. Drops the padding and clips, so the child's own
         * borders meet the card's radius instead of floating inside it.
         */
        flush: "gap-0 overflow-hidden py-0",
      },
    },
    defaultVariants: { variant: "default" },
  }
)

function Card({
  className,
  variant,
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof cardVariants>) {
  return (
    <div
      data-slot="card"
      data-variant={variant ?? "default"}
      className={cn(cardVariants({ variant }), className)}
      {...props}
    />
  )
}

function CardHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-header"
      className={cn(
        "@container/card-header grid auto-rows-min grid-rows-[auto_auto] items-start gap-2 px-6 has-data-[slot=card-action]:grid-cols-[1fr_auto] [.border-b]:pb-6",
        className
      )}
      {...props}
    />
  )
}

function CardTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-title"
      className={cn("leading-none font-semibold", className)}
      {...props}
    />
  )
}

function CardDescription({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  )
}

function CardAction({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-action"
      className={cn(
        "col-start-2 row-span-2 row-start-1 self-start justify-self-end",
        className
      )}
      {...props}
    />
  )
}

function CardContent({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-content"
      className={cn("px-6", className)}
      {...props}
    />
  )
}

function CardFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="card-footer"
      className={cn("flex items-center px-6 [.border-t]:pt-6", className)}
      {...props}
    />
  )
}

export {
  Card,
  CardHeader,
  CardFooter,
  CardTitle,
  CardAction,
  CardDescription,
  CardContent,
}
