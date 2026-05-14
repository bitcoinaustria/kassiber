import { useNavigate } from "@tanstack/react-router";
import * as React from "react";

export function TaxEvents() {
  const navigate = useNavigate();

  React.useEffect(() => {
    void navigate({ to: "/journals", replace: true });
  }, [navigate]);

  return (
    <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
      Opening journals...
    </div>
  );
}
