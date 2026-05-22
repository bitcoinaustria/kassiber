import { screenPanelClassName } from "@/lib/screen-layout";

interface QuarantineUnavailableProps {
  message?: string;
}

export function QuarantineUnavailable({ message }: QuarantineUnavailableProps) {
  return (
    <div className={screenPanelClassName}>
      <div className="rounded-xl border bg-card p-4">
        <h2 className="text-base font-semibold">Quarantine unavailable</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {message ?? "The daemon did not return quarantine data."}
        </p>
      </div>
    </div>
  );
}
