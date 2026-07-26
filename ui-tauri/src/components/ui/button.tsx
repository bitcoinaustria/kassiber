import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  // Press feedback on pointer-down (active), not click — keeps controls feeling
  // direct. Scale resets instantly when disabled so a dead control never dips.
  //
  // The `after` pseudo-element is an invisible hit-area floor, and the reason
  // it is sized in **px** rather than rem: the whole UI is rem-based off a root
  // font-size that ranges 10.24px–19.2px (16 × auto-fit × the user's scale, see
  // lib/appAutoScale.ts), so a rem-sized floor would shrink right along with
  // the thing it is supposed to be protecting. Without it, `sm` drops to 23px
  // and `icon-xs` to 17.3px at the auto-fit floor on a 13" laptop — under the
  // WCAG 2.2 AA 24×24 target minimum. It overlays the button centred, takes
  // pointer events (so it must NOT be `pointer-events-none`), and paints
  // nothing, so layout and appearance are untouched at every scale.
  "relative inline-flex shrink-0 items-center justify-center gap-2 rounded-md text-sm font-medium whitespace-nowrap transition-all outline-none after:absolute after:top-1/2 after:left-1/2 after:size-full after:min-h-[24px] after:min-w-[24px] after:-translate-x-1/2 after:-translate-y-1/2 after:content-[''] focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50 disabled:active:scale-100 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        // Solid fills carry a 1px top highlight and a soft drop shadow, and
        // flip to an inner top shadow while pressed — light from above, then
        // the surface sinks. The highlight only reads where the fill is dark
        // (light mode's ink `primary`, `destructive` in both themes); on dark
        // mode's near-white `primary` the drop shadow carries it alone.
        default:
          "bg-primary text-primary-foreground shadow-[inset_0_1px_0_rgb(255_255_255/0.16),0_1px_2px_0_rgb(0_0_0/0.10)] hover:bg-primary/90 active:shadow-[inset_0_1px_0_rgb(0_0_0/0.10)] disabled:shadow-none",
        destructive:
          "bg-destructive text-white shadow-[inset_0_1px_0_rgb(255_255_255/0.16),0_1px_2px_0_rgb(0_0_0/0.10)] hover:bg-destructive/90 focus-visible:ring-destructive/20 active:shadow-[inset_0_1px_0_rgb(0_0_0/0.10)] disabled:shadow-none dark:bg-destructive/60 dark:focus-visible:ring-destructive/40",
        // `bg-card`, not `bg-background`: in light mode `--background` is the
        // page itself, so an outline button on a page (47% of every button in
        // the app) was exactly the colour it sat on, with only a hairline to
        // separate it. `--card` is the raised surface, so it now reads as
        // sitting above the page. Dark mode swaps the drop shadow for a top
        // inset highlight — a black shadow over a near-black page is invisible,
        // so lighting the top edge is the only cue that works there.
        outline:
          "border bg-card shadow-xs hover:bg-accent hover:text-accent-foreground dark:border-input dark:bg-input/30 dark:shadow-[inset_0_1px_0_rgb(255_255_255/0.06)] dark:hover:bg-input/50",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        // Icons sit a step below the label unless the call site colours them
        // itself, so a row of ghost icon buttons reads as chrome rather than
        // competing with the content. This is what the app shell already
        // hand-rolled for its own icon buttons.
        ghost:
          "hover:bg-accent hover:text-accent-foreground dark:hover:bg-accent/50 [&_svg:not([class*='text-'])]:text-muted-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9 px-4 py-2 has-[>svg]:px-3",
        // h-7/size-7, not h-6/size-6: at the default scale that is 25.2px
        // instead of 21.6px, so the smallest controls clear the WCAG 2.2 AA
        // 24×24 target minimum on their own rather than leaning entirely on the
        // invisible floor in the base class.
        xs: "h-7 gap-1 rounded-md px-2 text-xs has-[>svg]:px-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-8 gap-1.5 rounded-md px-3 has-[>svg]:px-2.5",
        lg: "h-10 rounded-md px-6 has-[>svg]:px-4",
        icon: "size-9",
        "icon-xs": "size-7 rounded-md [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-8",
        "icon-lg": "size-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
