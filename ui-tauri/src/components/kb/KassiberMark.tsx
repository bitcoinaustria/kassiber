import { cn } from "@/lib/utils";

/*
 * The app icon's geometry (src-tauri/icons/icon.svg): the K and the Bitcoin
 * wedge tucked into its lower leg. Kept as paths rather than an <img> so the
 * mark stays crisp at any size.
 */
const K_PATH =
  "M862.75 150.9 644.25 153.2 324.55 484.4 319.95 155.5H129.05V870.8L317.65 873.1 319.95 739.7Z";
const WEDGE_PATH = "m405.05 735.1 128.8 138h361.1L582.15 548.8Z";
/** Bitcoin orange, as in the app icon. */
const WEDGE_FILL = "#f7931a";

/**
 * The app icon as a small tile beside the wordmark: the white K on its
 * near-black rounded square (`.kb-mark-tile`). Size it with `className`.
 */
export function KassiberMark({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "kb-mark-tile inline-flex shrink-0 items-center justify-center",
        className,
      )}
    >
      <svg viewBox="0 0 1024 1024" className="size-full">
        <path d={K_PATH} fill="#fff" />
        <path d={WEDGE_PATH} fill={WEDGE_FILL} />
      </svg>
    </span>
  );
}
