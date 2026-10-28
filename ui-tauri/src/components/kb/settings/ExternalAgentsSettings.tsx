import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { terminalCommandStatus, type TerminalCommandStatus } from "@/daemon/transport";
import {
  AGENT_GRANTS,
  RECOMMENDED_AGENT_ACCESS,
  RECOMMENDED_AGENT_GRANT,
  agentLauncher,
  claudeCodeCommand,
  mcpJsonConfig,
  mcpServeArgs,
  type AgentAccessMode,
  type AgentAccessStatus,
  type AgentGrant,
  type AgentPeer,
} from "@/lib/agentAccess";
import { cn } from "@/lib/utils";
import {
  CommandLine,
  CopyButton,
  SegmentedControl,
  SettingsGroup,
  SettingsRow,
} from "./SettingsControls";
import type { StatusData } from "./SettingsModel";

const GRANT_LABEL_KEYS = {
  read: "ai.agentsGrantRead",
  operator: "ai.agentsGrantOperator",
  accounting_decisions: "ai.agentsGrantDecisions",
} as const;
const GRANT_HINT_KEYS = {
  read: "ai.agentsGrantReadHint",
  operator: "ai.agentsGrantOperatorHint",
  accounting_decisions: "ai.agentsGrantDecisionsHint",
} as const;

/**
 * Off by default and subordinate to the AI master switch. The Python core
 * enforces both, and the grant, on every MCP call; this row only records the
 * user's choice and shows the command to paste into an agent.
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
  // Before agents are on, the choice is a draft the switch then records. A
  // first enable starts from the recommendation; a recorded choice is kept.
  const [draftGrant, setDraftGrant] = React.useState<AgentGrant | null>(null);
  const [draftAccess, setDraftAccess] = React.useState<AgentAccessMode | null>(null);

  const access =
    accessQuery.data?.kind === "ui.agent_access.status" ? accessQuery.data.data : null;
  const status = statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const enabled = aiFeaturesEnabled && access?.mcp_enabled === true;
  const recorded = access?.access_recorded === true;
  const grant: AgentGrant =
    access?.mcp_enabled === true
      ? (access.grant ?? "read")
      : (draftGrant ?? (recorded ? (access?.grant ?? "read") : RECOMMENDED_AGENT_GRANT));
  const agentAccess: AgentAccessMode =
    access?.mcp_enabled === true
      ? (access.agent_access ?? "restricted")
      : (draftAccess ??
        (recorded ? (access?.agent_access ?? "restricted") : RECOMMENDED_AGENT_ACCESS));
  // Once the core confirms agents are on, the draft is recorded: from then on
  // the recorded choice is what the row shows and a later enable re-sends.
  const recordedOn = access?.mcp_enabled === true;
  React.useEffect(() => {
    if (!recordedOn) return;
    setDraftGrant(null);
    setDraftAccess(null);
  }, [recordedOn]);
  const choiceDisabled = !aiFeaturesEnabled || !access || configure.isPending;
  const chooseGrant = (next: AgentGrant) => {
    if (access?.mcp_enabled === true) configure.mutate({ grant: next });
    else setDraftGrant(next);
  };
  const chooseAccess = (next: AgentAccessMode) => {
    if (access?.mcp_enabled === true) configure.mutate({ agent_access: next });
    else setDraftAccess(next);
  };

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
    <SettingsGroup
      title={t("ai.agentsLabel")}
      className={cn(!aiFeaturesEnabled && "opacity-60")}
    >
      <SettingsRow
        label={t("ai.agentsAllowLabel")}
        description={t("ai.agentsDescription")}
        htmlFor="settings-external-agents"
      >
        <Switch
          id="settings-external-agents"
          checked={enabled}
          disabled={!aiFeaturesEnabled || !access || configure.isPending}
          onCheckedChange={(checked) => {
            configure.mutate(
              checked
                ? {
                    mcp_enabled: true,
                    ai_features_enabled: aiFeaturesEnabled,
                    grant,
                    agent_access: agentAccess,
                  }
                : { mcp_enabled: false, ai_features_enabled: aiFeaturesEnabled },
            );
          }}
          aria-label={t("ai.agentsAria")}
        />
      </SettingsRow>
      {configure.isError ? (
        <p className="px-4 py-3 text-sm text-destructive">{t("ai.agentsError")}</p>
      ) : null}
      {aiFeaturesEnabled && access ? (
        <>
          <div className="space-y-2 px-4 py-3" data-agent-grant={grant}>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between sm:gap-6">
              <p id="settings-external-agents-grant" className="text-sm font-medium">
                {t("ai.agentsGrantLabel")}
              </p>
              <SegmentedControl<AgentGrant>
                label={t("ai.agentsGrantLabel")}
                value={grant}
                disabled={choiceDisabled}
                onValueChange={chooseGrant}
                options={AGENT_GRANTS.map((option) => ({
                  value: option,
                  label: t(GRANT_LABEL_KEYS[option]),
                }))}
              />
            </div>
            <p className="text-sm text-muted-foreground">{t(GRANT_HINT_KEYS[grant])}</p>
          </div>
          <SettingsRow
            label={t("ai.agentsRestrictLabel")}
            description={
              <span id="settings-external-agents-restrict-hint">
                {t("ai.agentsRestrictHint")}
              </span>
            }
            htmlFor="settings-external-agents-restrict"
          >
            <Switch
              id="settings-external-agents-restrict"
              checked={agentAccess === "restricted"}
              disabled={choiceDisabled}
              onCheckedChange={(checked) => chooseAccess(checked ? "restricted" : "full")}
              aria-describedby="settings-external-agents-restrict-hint"
            />
          </SettingsRow>
        </>
      ) : null}
      {enabled && args ? (
        <div className="flex items-center gap-2 px-4 py-3">
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
        <SettingsRow
          label={t("ai.agentsSessionLabel")}
          description={
            session.active
              ? t("ai.agentsSessionActive", { minutes: idleMinutes })
              : existingLease
                ? t("ai.agentsSessionExisting")
                : t("ai.agentsSessionLocked")
          }
        >
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={sessionBusy}
            onClick={() => (session.active ? lock.mutate(undefined) : unlock.mutate(undefined))}
          >
            {session.active ? t("ai.agentsSessionLock") : t("ai.agentsSessionUnlock")}
          </Button>
        </SettingsRow>
      ) : null}
      {session?.active ? (
        <div className="space-y-2 px-4 py-3">
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
        <p className="px-4 py-3 text-sm text-destructive">
          {t("ai.agentsSessionError")}
          {sessionError instanceof DaemonRequestError && sessionError.envelope.error?.hint
            ? ` ${sessionError.envelope.error.hint}`
            : null}
        </p>
      ) : null}
      {aiFeaturesEnabled && access ? (
        <p className="px-4 py-3 text-xs text-muted-foreground">
          {grant === "read" ? t("ai.agentsAdminNever") : t("ai.agentsHostApproval")}
        </p>
      ) : null}
    </SettingsGroup>
  );
}
