import { cn } from "@/lib/utils";

interface WordmarkProps {
  className?: string;
  size?: number;
}

export function Wordmark({ className, size = 22 }: WordmarkProps) {
  return (
    <span
      className={cn(
        "font-sans font-semibold text-ink leading-none tracking-tight",
        className,
      )}
      style={{ fontSize: size }}
    >
      Kassiber
    </span>
  );
}
