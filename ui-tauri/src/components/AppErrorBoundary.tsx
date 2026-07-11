import * as React from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import i18n from "@/i18n";
import {
  emitAppLog,
  exportLogRecords,
  getAppLogRecords,
} from "@/lib/appLogs";
import { appVersionLabel } from "@/lib/appVersion";
import { copyTextWithPolicy } from "@/lib/clipboard";
import { stackHead } from "@/lib/globalErrorCapture";

const COMPONENT_STACK_MAX_CHARS = 2000;
const DIAGNOSTIC_LOG_LIMIT = 200;

interface AppErrorBoundaryProps {
  children: React.ReactNode;
  /** Reset a route-local boundary after navigation succeeds. */
  resetKey?: string;
  /** Path captured in the React log record; query strings are excluded. */
  routePath?: string;
  /** Optional recovery action used by the route-local boundary. */
  onNavigateHome?: () => Promise<unknown> | unknown;
}

interface AppErrorBoundaryState {
  error: Error | null;
  copied: boolean;
  copyError: boolean;
}

export class AppErrorBoundary extends React.Component<
  AppErrorBoundaryProps,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = {
    error: null,
    copied: false,
    copyError: false,
  };

  static getDerivedStateFromError(error: Error): AppErrorBoundaryState {
    return { error, copied: false, copyError: false };
  }

  componentDidUpdate(previousProps: AppErrorBoundaryProps): void {
    if (
      this.state.error &&
      previousProps.resetKey !== this.props.resetKey
    ) {
      this.setState({ error: null, copied: false, copyError: false });
    }
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
        route: {
          type: "text",
          value: this.props.routePath ?? currentRoutePath(),
        },
      },
    });
  }

  private copyDiagnostics = async (): Promise<void> => {
    if (typeof navigator === "undefined" || !navigator.clipboard) {
      this.setState({ copyError: true, copied: false });
      return;
    }
    const generatedAt = new Date().toISOString();
    const records = getAppLogRecords().slice(-DIAGNOSTIC_LOG_LIMIT);
    const logText = exportLogRecords(records, "md", {
      redacted: true,
      header: {
        appVersion: appVersionLabel(),
        os: runtimeOsLabel(),
        timeRange: records.length
          ? `${records[0].ts} to ${records[records.length - 1].ts}`
          : "none",
        activeFilter: "error-boundary-recovery",
        redaction: "redacted",
        generatedAt,
      },
    });
    const diagnostics = [
      "# Kassiber crash diagnostics",
      "",
      `- Route: ${this.props.routePath ?? currentRoutePath()}`,
      `- Generated: ${generatedAt}`,
      "- The following records are redacted for clipboard sharing.",
      "",
      logText.trimEnd(),
      "",
    ].join("\n");
    try {
      await copyTextWithPolicy(diagnostics);
      this.setState({ copied: true, copyError: false });
    } catch {
      this.setState({ copyError: true, copied: false });
    }
  };

  private navigateHome = async (): Promise<void> => {
    if (!this.props.onNavigateHome) return;
    await this.props.onNavigateHome();
    this.setState({ error: null, copied: false, copyError: false });
  };

  render(): React.ReactNode {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6">
        <div className="w-full max-w-lg space-y-4 rounded-xl border bg-card p-6 text-center shadow-sm">
          <h1 className="text-lg font-semibold tracking-tight">
            {i18n.t("common:state.somethingWentWrong")}
          </h1>
          <p role="alert" className="text-sm break-words text-muted-foreground">
            {this.state.error.message || String(this.state.error)}
          </p>
          <div className="flex flex-wrap justify-center gap-2">
            {this.props.onNavigateHome ? (
              <Button
                type="button"
                variant="outline"
                onClick={() => void this.navigateHome()}
              >
                {i18n.t("chrome:errorBoundary.overview")}
              </Button>
            ) : null}
            <Button type="button" onClick={() => window.location.reload()}>
              {i18n.t("chrome:errorBoundary.reload")}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => void this.copyDiagnostics()}
            >
              {this.state.copied
                ? i18n.t("chrome:errorBoundary.copied")
                : i18n.t("chrome:errorBoundary.copy")}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {i18n.t("chrome:errorBoundary.details")}
          </p>
          {this.state.copyError ? (
            <p className="text-xs text-destructive">
              {i18n.t("chrome:errorBoundary.copyFailed")}
            </p>
          ) : null}
        </div>
      </div>
    );
  }
}

/**
 * Route-local boundary: a broken page must not take down AppShell navigation.
 * The pathname is the reset key, so a successful navigation clears the stale
 * error state before the next route renders.
 */
export function RouteErrorBoundary({ children }: { children: React.ReactNode }) {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const navigate = useNavigate();
  return (
    <AppErrorBoundary
      resetKey={pathname}
      routePath={pathname}
      onNavigateHome={() => navigate({ to: "/overview" })}
    >
      {children}
    </AppErrorBoundary>
  );
}

function currentRoutePath(): string {
  return typeof window === "undefined" ? "unknown" : window.location.pathname;
}

function runtimeOsLabel(): string {
  if (typeof navigator === "undefined") return "unknown";
  const nav = navigator as Navigator & {
    userAgentData?: { platform?: string };
  };
  return nav.userAgentData?.platform ?? nav.platform ?? "unknown";
}
