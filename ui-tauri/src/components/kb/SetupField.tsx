import type * as React from "react";

import { Label } from "@/components/ui/label";

export function SetupField({
  id,
  label,
  children,
  error,
  helper,
}: {
  id: string;
  label: string;
  children: React.ReactNode;
  error?: string;
  helper?: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {helper && !error ? renderSetupHelper(helper) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}

function renderSetupHelper(helper: React.ReactNode) {
  if (typeof helper === "string") {
    return <p className="text-xs text-muted-foreground">{helper}</p>;
  }
  return <div className="text-xs text-muted-foreground">{helper}</div>;
}
