import * as React from "react";

import { Button } from "@/components/ui/button";
import { emitAppLog } from "@/lib/appLogs";
import { stackHead } from "@/lib/globalErrorCapture";

const COMPONENT_STACK_MAX_CHARS = 2000;

interface AppErrorBoundaryProps {
  children: React.ReactNode;
}

interface AppErrorBoundaryState {
  error: Error | null;
}

export class AppErrorBoundary extends React.Component<
  AppErrorBoundaryProps,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): AppErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    emitAppLog({
      level: "error",
      module: "react",
      file: "components/AppErrorBoundary.tsx",
      line: 0,
      msg: error.message || String(error),
      fields: {
        component_stack: {
          type: "text",
          value: (info.componentStack ?? "").slice(0, COMPONENT_STACK_MAX_CHARS),
        },
        stack: { type: "text", value: stackHead(error) },
      },
    });
  }

  render(): React.ReactNode {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6">
        <div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-6 text-center shadow-sm">
          <h1 className="text-lg font-semibold tracking-tight">
            Something went wrong
          </h1>
          <p className="text-sm break-words text-muted-foreground">
            {this.state.error.message || String(this.state.error)}
          </p>
          <Button type="button" onClick={() => window.location.reload()}>
            Reload
          </Button>
          <p className="text-xs text-muted-foreground">
            Details are recorded under Developer tools &rarr; Logs.
          </p>
        </div>
      </div>
    );
  }
}
