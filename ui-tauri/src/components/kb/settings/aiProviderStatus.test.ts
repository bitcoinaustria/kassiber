import { describe, expect, it } from "vitest";

import {
  checkedAgo,
  hintSegments,
  providerEffortOptions,
  providerOffersFastMode,
  providerVersionLabel,
  runtimeSummary,
} from "./aiProviderStatus";
import type { AiProviderRow, AiProviderRuntimeStatus } from "@/lib/aiCapabilities";

function provider(base_url: string, extra: Partial<AiProviderRow> = {}): AiProviderRow {
  return {
    name: "p",
    base_url,
    kind: "remote",
    has_api_key: false,
    is_default: false,
    ...extra,
  };
}

describe("AI provider status helpers", () => {
  it("keeps the version number from a CLI's own wording", () => {
    expect(providerVersionLabel("2.1.287 (Claude Code)")).toBe("v2.1.287");
    expect(providerVersionLabel("GitHub Copilot CLI 1.0.93.")).toBe("v1.0.93");
    expect(providerVersionLabel("0.40.0-beta.2")).toBe("v0.40.0-beta.2");
    expect(providerVersionLabel("nightly")).toBe("nightly");
    expect(providerVersionLabel("  ")).toBeNull();
  });

  it("maps each broker state to its summary", () => {
    expect(runtimeSummary(null).headlineKey).toBe("notChecked");
    const base: Omit<AiProviderRuntimeStatus, "state" | "message"> = {
      provider: "copilot",
      display_name: "x",
      privacy_posture: "remote",
      native_tools: "disabled",
      models: [],
    };
    expect(runtimeSummary({ ...base, state: "ready", message: "Ready", version: "1.2.3" })).toEqual({
      tone: "ready", headlineKey: "ready", version: "v1.2.3", detail: null,
    });
    expect(runtimeSummary({ ...base, state: "missing_executable", message: "Install it." }).headlineKey).toBe("notInstalled");
    expect(runtimeSummary({ ...base, state: "authentication_required", message: "Run `copilot login`." })).toMatchObject({
      tone: "attention", headlineKey: "signInNeeded", detail: "Run `copilot login`.",
    });
    expect(runtimeSummary({ ...base, state: "error", message: "boom" }).tone).toBe("error");
  });

  it("splits hints into text and inline commands", () => {
    expect(hintSegments("Run `copilot login` outside Kassiber.")).toEqual([
      { code: false, text: "Run " },
      { code: true, text: "copilot login" },
      { code: false, text: " outside Kassiber." },
    ]);
  });

  it("offers a runtime's fixed efforts, else the advertised or all levels", () => {
    expect(providerEffortOptions(provider("claude-cli://default"), [])).toEqual([
      "low", "medium", "high", "xhigh", "max",
    ]);
    expect(
      providerEffortOptions(provider("codex-cli://default"), [
        { id: "a", reasoning_efforts: ["low", "high"] },
      ]),
    ).toEqual(["low", "high"]);
    expect(
      providerEffortOptions(
        provider("codex-cli://default", { default_reasoning_effort: "max" }),
        [{ id: "a", reasoning_efforts: ["low"] }],
      ),
    ).toEqual(["low", "max"]);
    expect(providerEffortOptions(provider("http://127.0.0.1:1/v1"), [])).toHaveLength(6);
  });

  it("mentions fast mode only where a model can offer it", () => {
    // Before a check nothing is known; the broker gates it on the CLI version.
    expect(providerOffersFastMode(provider("claude-cli://default"), [])).toBe(false);
    expect(
      providerOffersFastMode(provider("claude-cli://default"), [{ id: "opus", supports_fast_mode: true }]),
    ).toBe(true);
    expect(providerOffersFastMode(provider("codex-cli://default"), [])).toBe(false);
    expect(
      providerOffersFastMode(provider("codex-cli://default"), [{ id: "g", supports_fast_mode: true }]),
    ).toBe(true);
    expect(providerOffersFastMode(provider("copilot-cli://default"), [])).toBe(false);
    expect(
      providerOffersFastMode(provider("http://x/v1"), [{ id: "g", supports_fast_mode: true }]),
    ).toBe(false);
  });

  it("phrases check times relative to now", () => {
    const now = Date.parse("2026-10-09T10:00:00Z");
    expect(checkedAgo("2026-10-09T09:57:00Z", "en", now)).toBe("3 minutes ago");
    expect(checkedAgo("2026-10-09T09:59:50Z", "en", now)).toBe("now");
    expect(checkedAgo(null, "en", now)).toBeNull();
  });
});
