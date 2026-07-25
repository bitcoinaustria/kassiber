import { stdin } from "node:process";
import { codexChat, codexStatus } from "./codex.js";
import { claudeChat, claudeStatus } from "./claude.js";
import { openCodeChat, openCodeStatus } from "./opencode.js";
import type { BrokerRequest, ProviderId, ProviderStatus } from "./protocol.js";
import { safeErrorMessage, writeEvent } from "./protocol.js";
import { withWorkingDirectory } from "./working-directory.js";

const PROVIDERS: ProviderId[] = ["codex", "claude", "opencode"];

async function readRequest(): Promise<BrokerRequest> {
  let body = "";
  for await (const chunk of stdin) body += String(chunk);
  return JSON.parse(body) as BrokerRequest;
}

async function status(provider: ProviderId): Promise<ProviderStatus> {
  return withWorkingDirectory(provider, async (cwd) => {
    if (provider === "codex") return codexStatus(cwd);
    if (provider === "claude") return claudeStatus();
    return openCodeStatus(cwd);
  });
}

function errorCode(message: string) {
  if (/not installed|not found|enoent/i.test(message)) return "missing_executable" as const;
  if (/auth|login|unauthor|credential/i.test(message)) return "authentication_required" as const;
  return "provider_error" as const;
}

async function main(): Promise<void> {
  const request = await readRequest();
  if (request.command === "status") {
    writeEvent({
      type: "result",
      data: await Promise.all(PROVIDERS.map(status)),
    });
    return;
  }
  if (request.command === "models") {
    const snapshot = await status(request.provider);
    writeEvent({ type: "result", data: snapshot.models });
    return;
  }
  await withWorkingDirectory(request.provider, async (cwd) => {
    if (request.provider === "codex") await codexChat(request, cwd);
    else if (request.provider === "claude") await claudeChat(request, cwd);
    else await openCodeChat(request, cwd);
  });
}

main().catch((error) => {
  const message = safeErrorMessage(error);
  const code = errorCode(message);
  writeEvent({
    type: "error",
    code,
    message:
      code === "authentication_required"
        ? "The provider requires authentication."
        : message,
    hint:
      code === "authentication_required"
        ? "Run the provider's normal login command outside Kassiber."
        : undefined,
  });
  process.exitCode = 1;
});
