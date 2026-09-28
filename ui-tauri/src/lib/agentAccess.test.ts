import { describe, expect, it } from "vitest";

import type { TerminalCommandStatus } from "@/daemon/transport";

import { agentLauncher, claudeCodeCommand, mcpJsonConfig, mcpServeArgs } from "./agentAccess";

const terminal = (overrides: Partial<TerminalCommandStatus>): TerminalCommandStatus => ({
  platform: "linux",
  available: true,
  installed: false,
  managed: false,
  needsRepair: false,
  conflict: false,
  pathOnPath: false,
  command: "kassiber",
  binDir: "",
  commandPath: "",
  targetPath: "",
  pathHint: "",
  message: "",
  ...overrides,
});

describe("agentLauncher", () => {
  it("ignores a package-managed CLI the native status reports for macOS and Linux", () => {
    // The real native shape: another installation's command, installed, while
    // the app's own forwarding target is not the terminal command.
    expect(
      agentLauncher(
        terminal({
          platform: "macos",
          available: false,
          installed: true,
          pathOnPath: true,
          commandPath: "/opt/homebrew/bin/kassiber",
          targetPath: "/Applications/Kassiber.app/Contents/Resources/bin/kassiber",
        }),
      ),
    ).toEqual(["/Applications/Kassiber.app/Contents/Resources/bin/kassiber"]);
    expect(
      agentLauncher(
        terminal({
          available: false,
          installed: true,
          commandPath: "/usr/bin/kassiber",
          targetPath: "/home/u/Kassiber.AppImage",
        }),
      ),
    ).toEqual(["/home/u/Kassiber.AppImage", "--cli"]);
  });

  it("never prefers a kassiber on PATH, which may be another installation", () => {
    expect(
      agentLauncher(
        terminal({
          installed: true,
          pathOnPath: true,
          commandPath: "/opt/homebrew/bin/kassiber",
          targetPath: "/Applications/Kassiber.app/Contents/Resources/bin/kassiber",
        }),
      ),
    ).toEqual(["/Applications/Kassiber.app/Contents/Resources/bin/kassiber"]);
  });

  it("forwards through the app executable with --cli, by its stable path", () => {
    expect(agentLauncher(terminal({ targetPath: "/opt/Kassiber.AppImage" }))).toEqual([
      "/opt/Kassiber.AppImage",
      "--cli",
    ]);
    expect(
      agentLauncher(terminal({ targetPath: "/Applications/Kassiber.app/Contents/Resources/bin/kassiber" })),
    ).toEqual(["/Applications/Kassiber.app/Contents/Resources/bin/kassiber"]);
  });

  it("uses the installer's own command where the app has no forwarding target", () => {
    expect(
      agentLauncher(
        terminal({
          platform: "windows",
          available: false,
          installed: true,
          commandPath: "C:\\Program Files\\Kassiber\\bin\\kassiber.exe",
          targetPath: "C:\\Program Files\\Kassiber\\Kassiber.exe",
        }),
      ),
    ).toEqual(["C:\\Program Files\\Kassiber\\bin\\kassiber.exe"]);
  });

  it("falls back to kassiber outside the desktop app", () => {
    expect(agentLauncher(null)).toEqual(["kassiber"]);
    expect(agentLauncher(terminal({ available: false }))).toEqual(["kassiber"]);
  });
});

describe("agent snippets", () => {
  const args = mcpServeArgs({ dataRoot: "/data/my books", workspace: "Personal", profile: "Main" });

  it("pins the exact project and book", () => {
    expect(args).toEqual([
      "--data-root",
      "/data/my books",
      "mcp",
      "serve",
      "--workspace",
      "Personal",
      "--profile",
      "Main",
    ]);
  });

  it("quotes shell arguments for Claude Code", () => {
    expect(claudeCodeCommand(["kassiber"], args)).toBe(
      "claude mcp add kassiber -- kassiber --data-root '/data/my books' mcp serve --workspace Personal --profile Main",
    );
  });

  it("keeps launcher arguments in the JSON config", () => {
    const config = JSON.parse(mcpJsonConfig(["/opt/Kassiber.AppImage", "--cli"], args));
    expect(config.mcpServers.kassiber.command).toBe("/opt/Kassiber.AppImage");
    expect(config.mcpServers.kassiber.args.slice(0, 3)).toEqual(["--cli", "--data-root", "/data/my books"]);
  });
});
