import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

interface LabeledInputProps extends React.ComponentProps<"input"> {
  label: string;
  mono?: boolean;
}

export const LabeledInput = React.forwardRef<HTMLInputElement, LabeledInputProps>(
  function LabeledInput({ label, mono, className, id, ...rest }, ref) {
    const generatedId = React.useId();
    const inputId = id ?? generatedId;
    return (
      <div className="flex flex-col gap-2">
        <Label
          htmlFor={inputId}
          className="font-sans text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-2"
        >
          {label}
        </Label>
        <Input
          id={inputId}
          ref={ref}
          className={cn(
            "rounded-none border-line bg-paper text-sm",
            "focus-visible:border-ink focus-visible:ring-0",
            mono && "font-mono",
            className,
          )}
          {...rest}
        />
      </div>
    );
  },
);
