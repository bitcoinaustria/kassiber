import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { terminalCommandStatus, type TerminalCommandStatus } from "@/daemon/transport";
import {
  agentLauncher,
  claudeCodeCommand,
  mcpJsonConfig,
  mcpServeArgs,
  type AgentAccessStatus,
  type AgentPeer,
} from "@/lib/agentAccess";
import { cn } from "@/lib/utils";
import { CommandLine, CopyButton } from "./SettingsControls";
import type { StatusData } from "./SettingsModel";

/**
 * Off by default and subordinate to the AI master switch. The Python core
 * enforces both on every MCP call; this row only records the user's choice
 * and shows the command to paste into an agent.
 */
export function ExternalAgentsSettings({ aiFeaturesEnabled }: { aiFeaturesEnabled: boolean }) {
  const { t } = useTranslation("settings");
  const accessQuery = useDaemon<AgentAccessStatus>("ui.agent_access.status");
  const statusQuery = useDaemon<StatusData>("status");
  const configure = useDaemonMutation("ui.agent_access.configure");
  const unlock = useDaemonMutation<AgentAccessStatus>("ui.agent_access.unlock");
  const lock = useDaemonMutation("ui.agent_access.lock");
  const pairing = useDaemonMutation("ui.agent_access.pairing");
  const [terminal, setTerminal] = React.useState<TerminalCommandStatus | null>(null);

  const access =
    accessQuery.data?.kind === "ui.agent_access.status" ? accessQuery.data.data : null;
  const status = statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const enabled = aiFeaturesEnabled && access?.mcp_enabled === true;

  React.useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    void terminalCommandStatus()
      .then((result) => {
        if (!disposed) setTerminal(result);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, [enabled]);

  const book =
    status?.data_root && status.current_workspace && status.current_profile
      ? {
          dataRoot: status.data_root,
          workspace: status.current_workspace,
          profile: status.current_profile,
        }
      : null;
  const launcher = agentLauncher(terminal);
  const args = book ? mcpServeArgs(book) : null;
  // Only while this desktop's agent session exists (and this row is shown)
  // does the daemon ask the broker for waiting agents and activity.
  const liveQuery = useDaemon<AgentAccessStatus>(
    "ui.agent_access.status",
    { refresh: true },
    { enabled: access?.session?.active === true, refetchInterval: 3000, staleTime: 0 },
  );
  const live =
    liveQuery.data?.kind === "ui.agent_access.status" ? liveQuery.data.data : null;
  const session = access?.session?.active && live?.session ? live.session : access?.session;
  const agents = (session?.agents ?? []).filter((agent) => agent.state !== "denied");
  const idleMinutes = Math.round((session?.idle_timeout_seconds ?? 900) / 60);
  const sessionBusy = unlock.isPending || lock.isPending || pairing.isPending;
  const sessionError = unlock.error ?? lock.error ?? pairing.error;
  const decide = (agent: AgentPeer, allow: boolean) =>
    pairing.mutate({ session_id: agent.id, allow });
  const agentName = (agent: AgentPeer) =>
    agent.label ? `${agent.label} (${agent.pid})` : String(agent.pid);
  const existingLease =
    unlock.data?.kind === "ui.agent_access.unlock" && unlock.data.data?.session?.existing_lease === true;

  return (
    <div
      className={cn(
        "space-y-3 rounded-md border bg-background p-4",
        !aiFeaturesEnabled && "opacity-60",
      )}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0 space-y-1">
          <Label htmlFor="settings-external-agents">{t("ai.agentsLabel")}</Label>
          <p className="text-sm text-muted-foreground">{t("ai.agentsDescription")}</p>
        </div>
        <Switch
          id="settings-external-agents"
          checked={enabled}
          disabled={!aiFeaturesEnabled || !access || configure.isPending}
          onCheckedChange={(checked) =>
            configure.mutate({ mcp_enabled: checked, ai_features_enabled: aiFeaturesEnabled })
          }
          aria-label={t("ai.agentsAria")}
          className="shrink-0"
        />
      </div>
      {configure.isError ? (
        <p className="text-sm text-destructive">{t("ai.agentsError")}</p>
      ) : null}
      {enabled && args ? (
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1">
            <CommandLine command={claudeCodeCommand(launcher, args)} />
          </div>
          <span className="flex shrink-0 items-center text-xs text-muted-foreground">
            JSON
            <CopyButton value={mcpJsonConfig(launcher, args)} label={t("ai.agentsCopyJson")} />
          </span>
        </div>
      ) : null}
      {/* A lease the broker has not confirmed locking stays visible, with
          Lock, even after agents are turned off. */}
      {(enabled && session?.needed) || session?.active ? (
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-sm text-muted-foreground">
            {session.active
              ? t("ai.agentsSessionActive", { minutes: idleMinutes })
              : existingLease
                ? t("ai.agentsSessionExisting")
                : t("ai.agentsSessionLocked")}
          </p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="shrink-0"
            disabled={sessionBusy}
            onClick={() => (session.active ? lock.mutate(undefined) : unlock.mutate(undefined))}
          >
            {session.active ? t("ai.agentsSessionLock") : t("ai.agentsSessionUnlock")}
          </Button>
        </div>
      ) : null}
      {session?.active ? (
        <div className="space-y-2">
          {agents.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t("ai.agentsSessionWaiting")}</p>
          ) : null}
          {agents.map((agent) => (
            <div
              key={agent.id}
              className="flex flex-col gap-2 rounded-md border px-3 py-2 sm:flex-row sm:items-center sm:justify-between"
            >
              <p className="min-w-0 text-sm">
                {agent.state === "pending"
                  ? t("ai.agentsPairingRequest", { name: agentName(agent) })
                  : t("ai.agentsPairingAllowed", { name: agentName(agent), count: agent.calls })}
              </p>
              <div className="flex shrink-0 gap-2">
                {agent.state === "pending" ? (
                  <Button
                    type="button"
                    size="sm"
                    disabled={sessionBusy}
                    onClick={() => decide(agent, true)}
                  >
                    {t("ai.agentsPairingAllow")}
                  </Button>
                ) : null}
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={sessionBusy}
                  onClick={() => decide(agent, false)}
                >
                  {agent.state === "pending" ? t("ai.agentsPairingDeny") : t("ai.agentsPairingRemove")}
                </Button>
              </div>
            </div>
          ))}
        </div>
      ) : null}
      {(enabled || session?.active) && sessionError ? (
        <p className="text-sm text-destructive">
          {t("ai.agentsSessionError")}
          {sessionError instanceof DaemonRequestError && sessionError.envelope.error?.hint
            ? ` ${sessionError.envelope.error.hint}`
            : null}
        </p>
      ) : null}
    </div>
  );
}
