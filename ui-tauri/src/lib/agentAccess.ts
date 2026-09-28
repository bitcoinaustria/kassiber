import * as React from "react";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import type { TerminalCommandStatus } from "@/daemon/transport";

/** Global external-agent (MCP) access, read and enforced by the Python core. */
export interface AgentAccessStatus {
  mcp_enabled: boolean;
  ai_features_enabled: boolean | null;
  mcp_available: boolean;
  reason: "mcp_disabled" | "ai_features_disabled" | null;
}

export interface AgentBook {
  dataRoot: string;
  workspace: string;
  profile: string;
}

/**
 * The command an agent host should run: the installed terminal command when it
 * is on PATH, otherwise the app's own launcher (`--cli` forwards stdio to the
 * bundled CLI, so the host talks MCP straight to it).
 */
export function agentLauncher(terminal: TerminalCommandStatus | null): string[] {
  if (!terminal || !terminal.available) return ["kassiber"];
  if (terminal.installed && terminal.pathOnPath) return [terminal.command || "kassiber"];
  if (terminal.installed && terminal.commandPath) return [terminal.commandPath];
  if (!terminal.targetPath) return ["kassiber"];
  const name = terminal.targetPath.split(/[\\/]/).pop() ?? "";
  return /^kassiber(\.exe)?$/i.test(name)
    ? [terminal.targetPath]
    : [terminal.targetPath, "--cli"];
}

export function mcpServeArgs(book: AgentBook): string[] {
  return [
    "--data-root",
    book.dataRoot,
    "mcp",
    "serve",
    "--workspace",
    book.workspace,
    "--profile",
    book.profile,
  ];
}

function shellQuote(value: string): string {
  return /^[A-Za-z0-9_./:=@%+-]+$/.test(value)
    ? value
    : `'${value.replace(/'/g, `'\\''`)}'`;
}

export function claudeCodeCommand(launcher: string[], args: string[]): string {
  return ["claude", "mcp", "add", "kassiber", "--", ...launcher, ...args]
    .map(shellQuote)
    .join(" ");
}

export function mcpJsonConfig(launcher: string[], args: string[]): string {
  const [command, ...launcherArgs] = launcher;
  return JSON.stringify(
    { mcpServers: { kassiber: { command, args: [...launcherArgs, ...args] } } },
    null,
    2,
  );
}

/**
 * Mirror the AI master switch into the shared preference so `kassiber mcp`
 * refuses agents while AI features are off. Writes only when the stored value
 * differs, so an ordinary launch changes nothing on disk.
 */
export function useAgentAccessMasterSync(aiFeaturesEnabled: boolean, enabled: boolean) {
  const statusQuery = useDaemon<AgentAccessStatus>("ui.agent_access.status", undefined, {
    enabled,
    retry: false,
  });
  const configure = useDaemonMutation("ui.agent_access.configure");
  const stored =
    statusQuery.data?.kind === "ui.agent_access.status"
      ? statusQuery.data.data?.ai_features_enabled
      : undefined;
  const { mutate, isPending } = configure;
  React.useEffect(() => {
    if (!enabled || stored === undefined || isPending) return;
    // Unrecorded (`null`) blocks nothing, and turning agents on records it.
    if (stored === aiFeaturesEnabled || (stored === null && aiFeaturesEnabled)) return;
    mutate({ ai_features_enabled: aiFeaturesEnabled });
  }, [aiFeaturesEnabled, enabled, isPending, mutate, stored]);
}
