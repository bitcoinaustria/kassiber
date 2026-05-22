import * as React from "react";
import { ExternalLink } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  APP_LOG_MAX_BYTES,
  APP_LOG_MAX_RECORDS,
  getAppLogBufferSize,
  subscribeAppLogRecords,
} from "@/lib/appLogs";
import { SettingsSwitchRow } from "./SettingsControls";
import { formatBytes } from "./SettingsModel";

export function DeveloperToolsSettingsPanel({
  enabled,
  setEnabled,
  onOpenLogs,
}: {
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
  onOpenLogs: () => void;
}) {
  const bytes = useAppLogBufferSize();
  return (
    <section className="space-y-3">
      <p className="max-w-2xl text-sm text-muted-foreground">
        Show the typed Logs view after the local books are unlocked. Logs are
        local-only, kept in RAM, and written to disk only when you export them.
      </p>
      <SettingsSwitchRow
        label="Enable Logs page"
        description={
          enabled
            ? "Logs is visible in Support and route navigation."
            : "Logs is hidden and direct navigation redirects to Overview."
        }
        checked={enabled}
        onCheckedChange={setEnabled}
      />
      {enabled ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="w-fit"
          onClick={onOpenLogs}
        >
          <ExternalLink className="size-4" aria-hidden="true" />
          Open Logs
        </Button>
      ) : null}
      <div className="rounded-md border bg-background p-3 text-sm">
        <p className="font-medium">In-memory log buffer</p>
        <p className="text-muted-foreground">
          {formatBytes(bytes)} retained in this GUI session. Kassiber keeps at most{" "}
          {APP_LOG_MAX_RECORDS.toLocaleString()} records or{" "}
          {formatBytes(APP_LOG_MAX_BYTES)}, whichever is reached first. Refreshing
          or closing the app clears the buffer unless you export it first.
        </p>
      </div>
    </section>
  );
}

export function useAppLogBufferSize(): number {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogBufferSize,
    getAppLogBufferSize,
  );
}
