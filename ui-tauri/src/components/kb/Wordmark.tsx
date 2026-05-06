import { cn } from "@/lib/utils";

interface WordmarkProps {
  className?: string;
  size?: number;
}

export function Wordmark({ className, size = 22 }: WordmarkProps) {
  return (
    <span
      className={cn(
        "font-sans font-semibold leading-none tracking-tight text-foreground",
        className,
      )}
      style={{ fontSize: size }}
    >
      Kassiber
    </span>
  );
}
