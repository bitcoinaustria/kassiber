import * as React from "react";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import type { TerminalCommandStatus } from "@/daemon/transport";

/** One `kassiber mcp serve` process asking to use the desktop's session. */
export interface AgentPeer {
  id: string;
  /** The program that started it (for example `claude`); display only. */
  label: string | null;
  pid: number;
  state: "pending" | "allowed" | "denied";
  calls: number;
  last_call_at: string | null;
}

/**
 * Whether agents can read the open book. `needed` is true for an unlocked
 * encrypted book that agents cannot open on their own; `active` when this
 * desktop has unlocked it for them. Only a `refresh` read (while `active`)
 * asks the broker, which adds waiting agents and activity.
 */
export interface AgentSession {
  needed: boolean;
  active: boolean;
  expires_at: string | null;
  existing_lease?: boolean;
  idle_timeout_seconds?: number;
  calls?: number;
  last_call_at?: string | null;
  agents?: AgentPeer[];
}

/** Global external-agent (MCP) access, read and enforced by the Python core. */
export interface AgentAccessStatus {
  mcp_enabled: boolean;
  ai_features_enabled: boolean | null;
  mcp_available: boolean;
  reason: "mcp_disabled" | "ai_features_disabled" | null;
  session?: AgentSession;
}

export interface AgentBook {
  dataRoot: string;
  workspace: string;
  profile: string;
}

/**
 * The command an agent host should run: this desktop's own launcher, by its
 * stable absolute path (the AppImage file, the bundled CLI; `--cli` forwards
 * stdio to the bundled CLI). A `kassiber` found on PATH may be another
 * installation (Homebrew, a system package, another build), whose MCP server
 * this desktop's broker refuses, so it is never preferred.
 */
export function agentLauncher(terminal: TerminalCommandStatus | null): string[] {
  if (terminal?.platform === "windows") {
    // The installer's own CLI executable, beside this app.
    return terminal.installed && terminal.commandPath ? [terminal.commandPath] : ["kassiber"];
  }
  // Whether or not a terminal command is installed (and which one), agents
  // start this desktop's own target, the build whose broker they talk to.
  if (terminal?.targetPath) {
    const name = terminal.targetPath.split(/[\\/]/).pop() ?? "";
    return /^kassiber(\.exe)?$/i.test(name)
      ? [terminal.targetPath]
      : [terminal.targetPath, "--cli"];
  }
  // Outside the desktop app (browser preview, source runs).
  return ["kassiber"];
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
