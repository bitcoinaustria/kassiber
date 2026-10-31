// @vitest-environment happy-dom
//
// Mounted: the provider wizard's connection test is asynchronous, and what it
// may save depends on the values the form holds when the answer arrives.
// Every step stays mounted, so fields on a step out of view are still typed
// into directly; Test connection and Add provider are reached by walking the
// wizard with Next, which stops wherever a step does not hold up.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  mutations: {} as Record<string, ReturnType<typeof vi.fn>>,
}));

vi.hoisted(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke: mocks.invoke }),
}));

vi.mock("@/daemon/client", () => ({
  useDaemonMutation: (kind: string) => {
    mocks.mutations[kind] ??= vi.fn(async () => ({}));
    return { mutateAsync: mocks.mutations[kind], isPending: false };
  },
}));

import { AiProviderForm, type ExistingAiProvider } from "./AiProviderForm";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function testAnswer(data: Record<string, unknown>) {
  return {
    kind: "ai.test_connection",
    schema_version: 1,
    data: {
      model_count: 3,
      check_kind: "models",
      api_format: "auto",
      detected_api_format: null,
      tools_accepted: true,
      probe: { status: "ok", reason: null, model: "m", finish_reason: "stop" },
      ...data,
    },
  };
}

const ACK_NAME = /I understand prompts and selected Kassiber context may leave this device/;

function ackBox(): HTMLElement {
  return screen.getByRole("checkbox", { name: ACK_NAME, hidden: true });
}

/** Where each preset sits on the first step: its route card's title. */
const PRESET_ROUTES: Record<string, string> = {
  Ollama: "On this computer",
  oMLX: "On this computer",
  "LM Studio": "On this computer",
  vLLM: "On this computer",
  "llama.cpp": "On this computer",
  "Other compatible server": "On this computer",
  "Claude CLI": "Through your subscription app",
  "Codex CLI": "Through your subscription app",
  "OpenCode CLI": "Through your subscription app",
  "GitHub Copilot CLI": "Through your subscription app",
  OpenAI: "With an API key",
  OpenRouter: "With an API key",
  "Other OpenAI-compatible API": "With an API key",
};

function routeCard(title: string): HTMLElement {
  return screen.getByRole("radio", { name: new RegExp(`^${title}`), hidden: true });
}

/** Chooses a preset on the first step: its route, then the preset. */
function pick(label: string) {
  fireEvent.click(routeCard(PRESET_ROUTES[label]));
  fireEvent.click(screen.getByRole("radio", { name: label, hidden: true }));
}

function stepInView(): string | null {
  return (
    document
      .querySelector("[data-wizard-step]:not([hidden])")
      ?.getAttribute("data-wizard-step") ?? null
  );
}

/** Next, as the user presses it. */
function next() {
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
}

/** Presses Next until `step` is in view, or until a step holds the wizard. */
function walkTo(step: string) {
  for (let guard = 0; guard < 5 && stepInView() !== step; guard += 1) {
    const before = stepInView();
    const button = screen.queryByRole("button", { name: "Next" });
    if (!button) return;
    fireEvent.click(button);
    if (stepInView() === before) return;
  }
}

function errorText(): string {
  return screen.getByRole("alert").textContent ?? "";
}

function mount(initial: ExistingAiProvider | null = null) {
  const onClose = vi.fn();
  const onSaved = vi.fn();
  render(<AiProviderForm open initial={initial} onClose={onClose} onSaved={onSaved} />);
  return { onClose, onSaved };
}

function field(label: RegExp | string): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function type(label: RegExp | string, value: string) {
  fireEvent.change(field(label), { target: { value } });
}

function testCalls(): Array<Record<string, unknown>> {
  return mocks.invoke.mock.calls
    .map(([request]) => request as { kind: string; args: Record<string, unknown> })
    .filter((request) => request.kind === "ai.test_connection")
    .map((request) => request.args);
}

/** Walks to the last step and presses Test connection, if the wizard gets there. */
async function clickTest() {
  walkTo("check");
  const button = screen.queryByRole("button", { name: "Test connection" });
  if (!button) return;
  await act(async () => {
    fireEvent.click(button);
  });
}

/** Saves an edited provider from where it is; adds a new one from the last step. */
async function submit() {
  if (!screen.queryByRole("button", { name: "Save" })) walkTo("check");
  const button = screen.queryByRole("button", { name: /^(Add provider|Save)$/ });
  if (!button) return;
  await act(async () => {
    fireEvent.click(button);
  });
}

/** A new provider at another compatible server on this device. */
function startCustom() {
  mount();
  pick("Other compatible server");
}

describe("AiProviderForm (mounted)", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  it("drops a test answer for an endpoint the form no longer holds", async () => {
    const pending = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(pending.promise);
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    expect(testCalls()).toHaveLength(1);

    // Retargeted while the first endpoint is still answering.
    type("Base URL or CLI locator", "http://127.0.0.1:9090/v1");
    await act(async () => {
      pending.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.queryByText(/Connected/)).toBeNull();

    await submit();
    const create = mocks.mutations["ai.providers.create"];
    expect(create).toHaveBeenCalledOnce();
    const args = create.mock.calls[0][0] as Record<string, unknown>;
    expect(args.base_url).toBe("http://127.0.0.1:9090/v1");
    // The other endpoint's detection is not saved with this one.
    expect(args).not.toHaveProperty("detected_api_format");
  });

  it("saves the detection of a test that matches what is saved", async () => {
    mocks.invoke.mockResolvedValueOnce(testAnswer({ detected_api_format: "chat_completions" }));
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    expect(screen.getByText(/Connected/)).toBeTruthy();

    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<
      string,
      unknown
    >;
    expect(args.detected_api_format).toBe("chat_completions");
  });

  it("ignores an older run's answer once a newer run started", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    type("Base URL or CLI locator", "http://127.0.0.1:9090/v1");
    await clickTest();

    await act(async () => {
      first.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.getByText("Testing…")).toBeTruthy();
    await act(async () => {
      second.resolve(testAnswer({ detected_api_format: "responses" }));
    });
    expect(screen.getByText(/Connected/)).toBeTruthy();

    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<
      string,
      unknown
    >;
    expect(args.detected_api_format).toBe("responses");
  });

  it("reports only the latest of two runs for the same values", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    // A double click: the same values, twice.
    await clickTest();
    await clickTest();
    expect(testCalls()).toHaveLength(2);

    // The first run fails late; the second is still the one that counts.
    await act(async () => {
      first.resolve({
        kind: "error",
        schema_version: 1,
        error: { code: "ai_unavailable", message: "first run timed out", retryable: true },
      });
    });
    expect(screen.queryByText(/first run timed out/)).toBeNull();
    expect(screen.getByText("Testing…")).toBeTruthy();

    await act(async () => {
      second.resolve(testAnswer({ detected_api_format: "chat_completions" }));
    });
    expect(screen.getByText(/Connected/)).toBeTruthy();
    await submit();
    expect(
      (mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<string, unknown>)
        .detected_api_format,
    ).toBe("chat_completions");
  });

  it("keeps the latest run's failure over an older run's success", async () => {
    const first = deferred<unknown>();
    const second = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    type("Default model (optional)", "m");
    await clickTest();
    await clickTest();
    await act(async () => {
      second.resolve({
        kind: "error",
        schema_version: 1,
        error: { code: "ai_auth_failed", message: "key revoked", retryable: false },
      });
    });
    await act(async () => {
      first.resolve(testAnswer({ detected_api_format: "responses" }));
    });
    expect(screen.getByText("Test failed: key revoked")).toBeTruthy();
    expect(screen.queryByText(/Connected/)).toBeNull();
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).not.toHaveProperty(
      "detected_api_format",
    );
  });
});

function posture(): string | null {
  const pressed = screen
    .getAllByRole("button", { name: /^(local|remote|tee)$/, hidden: true })
    .find((button) => button.getAttribute("aria-pressed") === "true");
  return pressed?.textContent ?? null;
}

const OFF_DEVICE_NOTE = "This address isn't on this device, so it's treated as remote.";

describe("AiProviderForm posture from the URL", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  it("moves a local provider to remote for an address off this device, and says so", () => {
    startCustom();
    expect(posture()).toBe("local");
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // Nothing moves while the address is still being typed.
    expect(posture()).toBe("local");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByRole("status", { hidden: true }).textContent).toBe(OFF_DEVICE_NOTE);
    // The off-device acknowledgement is asked for, not assumed.
    expect(
      (ackBox() as HTMLButtonElement).getAttribute("aria-checked"),
    ).toBe("false");

    // Back to an address on this device: the note goes, the posture stays.
    type("Base URL or CLI locator", "http://127.0.0.1:8000/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
    expect(posture()).toBe("remote");
  });

  it("keeps local for this device's addresses", () => {
    startCustom();
    for (const url of ["http://localhost:11434/v1", "http://127.0.0.1:8000/v1", "http://[::1]:8000/v1"]) {
      type("Base URL or CLI locator", url);
      fireEvent.blur(field("Base URL or CLI locator"));
      expect(posture()).toBe("local");
    }
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
  });

  it("never moves the user's remote or TEE choice", () => {
    startCustom();
    fireEvent.click(screen.getByRole("button", { name: "remote", hidden: true }));
    type("Base URL or CLI locator", "http://localhost:11434/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");

    fireEvent.click(screen.getByRole("button", { name: "tee", hidden: true }));
    type("Base URL or CLI locator", "https://enclave.example.test/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("tee");
    expect(screen.queryByText(OFF_DEVICE_NOTE)).toBeNull();
  });

  it("treats a CLI agent as remote", () => {
    startCustom();
    type("Base URL or CLI locator", "claude-cli://default");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByRole("status", { hidden: true }).textContent).toBe(
      "CLI agents can send prompts off this device, so this provider is treated as remote.",
    );
  });

  it("asks for the acknowledgement once the address moves the posture", async () => {
    startCustom();
    next();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // Enter straight from the field, without leaving it.
    await act(async () => {
      fireEvent.submit(field("Base URL or CLI locator").closest("form")!);
    });
    expect(posture()).toBe("remote");
    expect(stepInView()).toBe("privacy");
    expect(ackBox().getAttribute("aria-checked")).toBe("false");
    next();
    expect(stepInView()).toBe("privacy");
    expect(errorText()).toBe("Acknowledge that prompts may leave this device to continue.");
    await submit();
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();
  });

  it("refuses to save when the address moved the posture after the privacy step", async () => {
    startCustom();
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    walkTo("check");
    expect(stepInView()).toBe("check");
    // Retargeted off this device without walking the wizard again.
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    await submit();
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();
    expect(stepInView()).toBe("privacy");
    expect(errorText()).toBe("Acknowledge that prompts may leave this device before saving.");
  });

  it("tests an address off this device as remote, once acknowledged", async () => {
    mocks.invoke.mockResolvedValueOnce(testAnswer({}));
    startCustom();
    type("Name", "Gateway");
    type("Base URL or CLI locator", "https://gateway.example.test/v1");
    // The posture moves and the wizard waits for the acknowledgement it now
    // needs.
    await clickTest();
    expect(stepInView()).toBe("privacy");
    expect(posture()).toBe("remote");
    expect(testCalls()).toEqual([]);
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[0]).toMatchObject({
      base_url: "https://gateway.example.test/v1",
      kind: "remote",
      acknowledged: true,
    });
  });
});

describe("AiProviderForm off-device acknowledgement for Test connection", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(testAnswer({}));
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const ACK_FIRST =
    "Testing sends a short prompt to this provider. Acknowledge that prompts may leave this device first.";

  it("does not test a remote preset until the box is ticked", async () => {
    mount();
    pick("OpenRouter");
    type("API key (required)", "sk-or-test");
    await clickTest();
    expect(stepInView()).toBe("privacy");
    expect(errorText()).toBe("Acknowledge that prompts may leave this device to continue.");
    expect(mocks.invoke).not.toHaveBeenCalled();

    fireEvent.click(ackBox());
    await clickTest();
    expect(screen.queryByText(ACK_FIRST)).toBeNull();
    expect(testCalls()).toEqual([
      expect.objectContaining({
        base_url: "https://openrouter.ai/api/v1",
        kind: "remote",
        api_key: "sk-or-test",
        acknowledged: true,
      }),
    ]);
  });

  it("needs no box for a local server on this device", async () => {
    startCustom();
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    await clickTest();
    // A local server has no privacy step.
    expect(screen.getByRole("navigation").textContent).not.toContain("Privacy");
    expect(testCalls()).toHaveLength(1);
    expect(testCalls()[0]).not.toHaveProperty("acknowledged");
  });

  it("relies on a saved off-device provider's own acknowledgement", async () => {
    mount({
      name: "gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
    });
    expect(screen.queryByRole("checkbox", { name: ACK_NAME })).toBeNull();
    await clickTest();
    expect(testCalls()).toHaveLength(1);
    expect(testCalls()[0]).not.toHaveProperty("acknowledged");
  });

  it("asks again when a saved local provider becomes remote", async () => {
    mount({
      name: "box",
      base_url: "http://127.0.0.1:1234/v1",
      kind: "local",
      has_api_key: false,
      acknowledged_at: "2026-10-01T00:00:00Z",
    });
    fireEvent.click(screen.getByRole("button", { name: "remote", hidden: true }));
    await clickTest();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(stepInView()).toBe("privacy");
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ kind: "remote", acknowledged: true });
  });
});

describe("AiProviderForm presets", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const ACK = /I understand prompts and selected Kassiber context may leave this device/;

  const choose = pick;

  it.each([
    {
      label: "Ollama",
      name: "Ollama",
      url: "http://localhost:11434/v1",
      posture: "local",
      model: "qwen3.6:35b",
      notes: "Local Ollama endpoint.",
      hint: "Runs models on this device. Start it with:ollama serve",
      keyHint: "(usually not required)",
    },
    {
      label: "LM Studio",
      name: "LM Studio",
      url: "http://localhost:1234/v1",
      posture: "local",
      model: "",
      notes: "Local LM Studio endpoint.",
      hint: "Start the server in LM Studio's Developer tab, or with:lms server start",
      keyHint: "(usually not required)",
    },
    {
      label: "vLLM",
      name: "vLLM",
      url: "http://127.0.0.1:8000/v1",
      posture: "local",
      model: "",
      notes: "Local vLLM endpoint.",
      hint: /--enable-auto-tool-choice --tool-call-parser hermes$/,
      keyHint: "(usually not required)",
    },
    {
      label: "llama.cpp",
      name: "llama.cpp",
      url: "http://127.0.0.1:8080/v1",
      posture: "local",
      model: "",
      notes: "Local llama.cpp endpoint.",
      hint: /only with --jinja\. Start it with:llama-server -m <model\.gguf> --jinja$/,
      keyHint: "(usually not required)",
    },
    {
      label: "OpenAI",
      name: "OpenAI",
      url: "https://api.openai.com/v1",
      posture: "remote",
      model: "",
      notes: "OpenAI; prompts may leave this device.",
      hint: "OpenAI's own models, with an API key from your OpenAI account.",
      keyHint: "(required)",
    },
    {
      label: "OpenRouter",
      name: "OpenRouter",
      url: "https://openrouter.ai/api/v1",
      posture: "remote",
      model: "",
      notes: "OpenRouter; prompts may leave this device.",
      hint: "One key for models from many vendors.",
      keyHint: "(required)",
    },
    {
      label: "Other compatible server",
      name: "",
      url: "",
      posture: "local",
      model: "",
      notes: "",
      hint: /Any other server that speaks the OpenAI API/,
      keyHint: "(usually not required)",
    },
    {
      label: "Other OpenAI-compatible API",
      name: "",
      url: "",
      posture: "remote",
      model: "",
      notes: "",
      hint: /Any hosted service that speaks the OpenAI API/,
      keyHint: "(optional for keyless deployments)",
    },
    {
      label: "Codex CLI",
      name: "Codex CLI",
      url: "codex-cli://default",
      posture: "remote",
      model: "default",
      notes: "Codex CLI; prompts may leave this device.",
      hint: "Uses the Codex CLI you're signed in to; it can send prompts to its own model service.",
      keyHint: "(optional for keyless deployments)",
    },
  ])("$label fills the form", ({ label, name, url, posture: expected, model, notes, hint, keyHint }) => {
    mount();
    // Start from another preset, so every field is seen to change.
    choose(label === "Ollama" ? "oMLX" : "Ollama");
    choose(label);
    expect(field("Name").value).toBe(name);
    expect(field("Base URL or CLI locator").value).toBe(url);
    expect(field("Default model (optional)").value).toBe(model);
    expect(field("Notes (optional)").value).toBe(notes);
    expect(posture()).toBe(expected);
    expect(screen.getByRole("radio", { name: label }).getAttribute("aria-checked")).toBe("true");
    expect(
      document.querySelector("[data-preset-hint]")?.textContent,
    ).toEqual(typeof hint === "string" ? hint : expect.stringMatching(hint));
    expect(screen.getByText(keyHint)).toBeTruthy();
    // A remote preset shows the off-device warning and asks for the
    // acknowledgement; a local one does not.
    expect(Boolean(screen.queryByText(ACK))).toBe(expected === "remote");
  });

  it("offers a local server's start command to copy, and nothing for a hosted one", () => {
    mount();
    choose("vLLM");
    expect(document.querySelector("[data-preset-command]")?.textContent).toBe(
      "vllm serve <model> --enable-auto-tool-choice --tool-call-parser hermes",
    );
    expect(screen.getByRole("button", { name: "Copy command" })).toBeTruthy();
    choose("OpenAI");
    expect(document.querySelector("[data-preset-command]")).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy command" })).toBeNull();
  });

  it("groups the presets by where prompts go", () => {
    mount();
    const offered = () =>
      Array.from(document.querySelectorAll("[data-provider-preset]")).map(
        (option) => option.getAttribute("data-provider-preset"),
      );
    // Nothing to choose until the route is.
    expect(offered()).toEqual([]);
    expect(screen.getByRole("button", { name: "Next" })).toHaveProperty("disabled", true);
    const routes = screen.getAllByRole("radio").map((card) => card.getAttribute("data-provider-route"));
    expect(routes).toEqual(["local", "app", "api"]);
    expect(
      routeCard("On this computer").querySelector("[data-posture]")?.getAttribute("data-posture"),
    ).toBe("local");

    fireEvent.click(routeCard("On this computer"));
    expect(offered()).toEqual([
      "ollama",
      "lmstudio",
      "vllm",
      "llamacpp",
      "omlx",
      "openai-compatible",
    ]);
    fireEvent.click(routeCard("Through your subscription app"));
    expect(offered()).toEqual(["claude-cli", "codex-cli", "opencode-cli", "copilot-cli"]);
    expect(
      routeCard("Through your subscription app").querySelector("[data-posture]")?.getAttribute(
        "data-posture",
      ),
    ).toBe("remote");
    fireEvent.click(routeCard("With an API key"));
    expect(offered()).toEqual(["openai", "openrouter", "openai-compatible-api"]);
    // Choosing one moves the wizard on with its values.
    choose("OpenAI");
    next();
    expect(stepInView()).toBe("connect");
    expect(field("Base URL or CLI locator").value).toBe("https://api.openai.com/v1");
  });

  it("needs the key for OpenRouter before saving", async () => {
    mount();
    choose("OpenRouter");
    fireEvent.click(ackBox());
    await submit();
    expect(stepInView()).toBe("connect");
    expect(errorText()).toBe("Enter your OpenRouter API key before saving.");
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();

    type("API key (required)", "sk-or-test");
    await submit();
    expect(mocks.mutations["ai.providers.create"]).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        kind: "remote",
        acknowledged: true,
      }),
    );
    expect(mocks.mutations["ai.providers.set_api_key"]).toHaveBeenCalledWith({
      name: "openrouter",
      api_key: "sk-or-test",
    });
  });

  it("follows the URL's posture for another OpenAI-compatible server", () => {
    mount();
    choose("OpenAI");
    choose("Other compatible server");
    expect(posture()).toBe("local");
    type("Base URL or CLI locator", "https://llm.example.test/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(posture()).toBe("remote");
    expect(screen.getByText(OFF_DEVICE_NOTE)).toBeTruthy();
  });
});

describe("AiProviderForm API format and test results", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  function local() {
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    type("Default model (optional)", "qwen3:8b");
  }

  function result(): HTMLElement {
    const element = document.querySelector<HTMLElement>("[data-test-result]");
    if (!element) throw new Error("no test result");
    return element;
  }

  it("sends the chosen format with the test and the saved row", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ api_format: "chat_completions", detected_api_format: "chat_completions" }));
    startCustom();
    local();
    expect(screen.getByText("API format: Auto (recommended)")).toBeTruthy();
    const chat = screen.getByRole("radio", { name: "Chat Completions", hidden: true });
    fireEvent.click(chat);
    expect((chat as HTMLInputElement).checked).toBe(true);
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ api_format: "chat_completions", model: "qwen3:8b" });
    expect(result().textContent).toContain("Format: Chat Completions (as set)");

    await submit();
    expect(mocks.mutations["ai.providers.create"]).toHaveBeenCalledWith(
      expect.objectContaining({
        api_format: "chat_completions",
        detected_api_format: "chat_completions",
      }),
    );
  });

  it("tests and saves auto by default, and leaves the format out for a CLI agent", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({}));
    startCustom();
    local();
    await clickTest();
    expect(testCalls()[0].api_format).toBe("auto");

    type("Base URL or CLI locator", "codex-cli://default");
    // A CLI agent takes no API format.
    expect(screen.queryByRole("radiogroup", { name: "API format", hidden: true })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "remote", hidden: true }));
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[1]).not.toHaveProperty("api_format");
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).not.toHaveProperty(
      "api_format",
    );
  });

  it("forgets a test result once the format changes", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ detected_api_format: "chat_completions" }));
    startCustom();
    local();
    await clickTest();
    expect(result().textContent).toContain("Format: Chat Completions (detected)");
    fireEvent.click(screen.getByRole("radio", { name: "Responses API", hidden: true }));
    expect(document.querySelector("[data-test-result]")).toBeNull();
    await submit();
    const args = mocks.mutations["ai.providers.create"].mock.calls[0][0] as Record<string, unknown>;
    expect(args.api_format).toBe("responses");
    expect(args).not.toHaveProperty("detected_api_format");
  });

  it("shows what answered, over which format, and that tools were accepted", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 12,
        detected_api_format: "chat_completions",
        tools_accepted: true,
        probe: { status: "ok", reason: null, model: "qwen3:8b", finish_reason: "stop" },
      }),
    );
    startCustom();
    local();
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("ok");
    const text = result().textContent ?? "";
    expect(text).toContain("Connected — 12 models reachable.");
    expect(text).toContain("qwen3:8b answered the test prompt.");
    expect(text).toContain("Format: Chat Completions (detected)");
    expect(text).toContain("Tools accepted.");
    expect(document.querySelector("[data-tools-refused]")).toBeNull();
  });

  it("says when the endpoint refused tools", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({ detected_api_format: "responses", tools_accepted: false }),
    );
    startCustom();
    local();
    await clickTest();
    expect(document.querySelector("[data-tools-refused]")?.textContent).toBe(
      "This endpoint answered but refused tools, so the assistant can't read your book through it.",
    );
    expect(result().textContent).not.toContain("Tools accepted.");
  });

  it("does not call a listing without a reply a success", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 342,
        detected_api_format: null,
        tools_accepted: null,
        probe: { status: "skipped", reason: "no_model", model: null, finish_reason: null },
      }),
    );
    startCustom();
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("no_model");
    expect(result().textContent).toContain(
      "Connected — 342 models listed, but no reply was checked.",
    );
    expect(result().textContent).toContain("Choose a model to check that it can answer.");
    expect(result().textContent).not.toContain("reachable");
  });

  it("shows a failed test plainly", async () => {
    mocks.invoke.mockResolvedValue({
      kind: "error",
      schema_version: 1,
      error: {
        code: "ai_auth_failed",
        message: "AI provider rejected the request as unauthorized",
        retryable: false,
      },
    });
    startCustom();
    local();
    await clickTest();
    expect(result().getAttribute("data-test-result")).toBe("fail");
    expect(result().textContent).toBe(
      "Test failed: AI provider rejected the request as unauthorized",
    );
  });

  it("resets the format when a pinned HTTP provider becomes a CLI agent", async () => {
    mount({
      name: "router",
      display_name: "Router",
      base_url: "https://openrouter.ai/api/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "responses",
      detected_api_format: "responses",
    });
    type("Base URL or CLI locator", "codex-cli://default");
    // A CLI agent takes no API format.
    expect(screen.queryByRole("radiogroup", { name: "API format", hidden: true })).toBeNull();
    await submit();
    const update = mocks.mutations["ai.providers.update"];
    expect(update).toHaveBeenCalledOnce();
    expect(update.mock.calls[0][0]).toMatchObject({
      name: "router",
      base_url: "codex-cli://default",
      api_format: "auto",
      clear: ["detected_api_format"],
    });
    expect(update.mock.calls[0][0]).not.toHaveProperty("detected_api_format");
  });

  it("opens Advanced on a saved non-auto format and names the last detection", () => {
    mount({
      name: "gateway",
      display_name: "Gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "auto",
      detected_api_format: "chat_completions",
    });
    expect(screen.getByText("Last detected: Chat Completions")).toBeTruthy();
    expect(
      (screen.getByRole("radio", { name: "Auto (recommended)", hidden: true }) as HTMLInputElement).checked,
    ).toBe(true);
    // The detection belongs to the saved URL.
    type("Base URL or CLI locator", "https://other.example.test/v1");
    expect(screen.queryByText("Last detected: Chat Completions")).toBeNull();
    cleanup();

    mount({
      name: "gateway",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      api_format: "responses",
    });
    expect(
      (document.querySelector("[data-provider-advanced]") as HTMLDetailsElement).open,
    ).toBe(true);
    expect((screen.getByRole("radio", { name: "Responses API", hidden: true }) as HTMLInputElement).checked).toBe(
      true,
    );
  });
});

describe("AiProviderForm model suggestions", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  function suggestions(): string[] {
    const listId = field("Default model (optional)").getAttribute("list");
    if (!listId) return [];
    return Array.from(document.querySelectorAll(`#${listId} option`)).map(
      (option) => (option as HTMLOptionElement).value,
    );
  }

  it("offers the models the test listed that can run the assistant", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({
        model_count: 4,
        models: [
          { id: "gpt-5", supports_tools: null },
          { id: "text-embedding-3-small", supports_tools: null },
          { id: "acme/no-tools", supports_tools: false },
          { id: "gpt-4o", supports_tools: null },
        ],
        detected_api_format: null,
        tools_accepted: null,
        probe: { status: "skipped", reason: "no_model", model: null, finish_reason: null },
      }),
    );
    startCustom();
    type("Name", "OpenAI");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    expect(suggestions()).toEqual([]);
    await clickTest();
    expect(suggestions()).toEqual(["gpt-5", "gpt-4o"]);
    expect(screen.getByText("Type to choose from the 2 models the test listed.")).toBeTruthy();

    // Choosing one keeps the list; another endpoint drops it.
    type("Default model (optional)", "gpt-5");
    expect(suggestions()).toEqual(["gpt-5", "gpt-4o"]);
    type("Base URL or CLI locator", "http://127.0.0.1:9999/v1");
    expect(suggestions()).toEqual([]);
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });
});

describe("AiProviderForm no-retention flag", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const OMIT_NAME = /Send requests without Kassiber's no-retention flag/;

  function omitBox(): HTMLElement {
    return screen.getByRole("checkbox", { name: OMIT_NAME, hidden: true });
  }

  function local() {
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    type("Default model (optional)", "m");
  }

  it("keeps the flag unless the user explicitly ticks the option", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ store_accepted: true }));
    startCustom();
    local();
    expect(omitBox().getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText(/the provider's own retention applies to your prompts/)).toBeTruthy();
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ omit_store_field: false });
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).toMatchObject({
      omit_store_field: false,
    });
  });

  it("does not let a reselected preset carry the exemption to the next address", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ store_accepted: null }));
    mount();
    const otherPreset = () => pick("Other compatible server");
    otherPreset();
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    fireEvent.click(omitBox());
    expect(omitBox().getAttribute("aria-checked")).toBe("true");
    // The same preset again replaces the address: the exemption chosen for
    // endpoint A must not become the "initial" choice for endpoint B.
    otherPreset();
    type("Name", "Box");
    type("Base URL or CLI locator", "http://127.0.0.1:5678/v1");
    type("Default model (optional)", "m");
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).toMatchObject({
      base_url: "http://127.0.0.1:5678/v1",
      omit_store_field: false,
    });
  });

  it("sends requests without the flag once ticked, and says so", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ store_accepted: null }));
    startCustom();
    local();
    fireEvent.click(omitBox());
    expect(omitBox().getAttribute("aria-checked")).toBe("true");
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ omit_store_field: true });
    expect(document.querySelector("[data-store-omitted]")?.textContent).toBe(
      "Sent without Kassiber's no-retention flag; the provider's own retention applies.",
    );
    await submit();
    expect(mocks.mutations["ai.providers.create"].mock.calls[0][0]).toMatchObject({
      omit_store_field: true,
    });
  });

  it("explains an endpoint that refuses the flag, and opens Advanced without ticking it", async () => {
    mocks.invoke.mockResolvedValue(
      testAnswer({ store_accepted: false, detected_api_format: "chat_completions" }),
    );
    startCustom();
    local();
    const advanced = document.querySelector("[data-provider-advanced]") as HTMLDetailsElement;
    expect(advanced.open).toBe(false);
    await clickTest();
    expect(document.querySelector("[data-store-refused]")?.textContent).toBe(
      "This endpoint refuses Kassiber's no-retention flag (store: false), so chats with it will fail. Under Advanced you can send requests without the flag; the provider's own retention then applies.",
    );
    expect(advanced.open).toBe(true);
    expect(omitBox().getAttribute("aria-checked")).toBe("false");
    // Ticking it changes what is tested, so the old result goes.
    fireEvent.click(omitBox());
    expect(document.querySelector("[data-store-refused]")).toBeNull();
  });

  it("shows a saved choice, and leaves it out for a CLI agent", async () => {
    mount({
      name: "gw",
      base_url: "https://llm.example.test/v1",
      kind: "remote",
      has_api_key: true,
      acknowledged_at: "2026-10-01T00:00:00Z",
      omit_store_field: true,
    });
    expect((document.querySelector("[data-provider-advanced]") as HTMLDetailsElement).open).toBe(
      true,
    );
    expect(omitBox().getAttribute("aria-checked")).toBe("true");
    type("Base URL or CLI locator", "codex-cli://default");
    expect(screen.queryByRole("checkbox", { name: OMIT_NAME })).toBeNull();
    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).not.toHaveProperty(
      "omit_store_field",
    );
  });
});

describe("AiProviderForm keys belong to their address", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(testAnswer({ models: [{ id: "openai/gpt-5" }] }));
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const CLEARED = "API key cleared because the address changed.";

  const choose = pick;

  function typeKey(value: string) {
    fireEvent.change(document.getElementById("ai-form-key") as HTMLInputElement, {
      target: { value },
    });
  }

  function keyValue(): string {
    return (document.getElementById("ai-form-key") as HTMLInputElement).value;
  }

  it("clears the key, format, store choice and test result on another preset", async () => {
    mount();
    choose("OpenAI");
    typeKey("sk-openai-secret");
    fireEvent.click(screen.getByRole("radio", { name: "Chat Completions", hidden: true }));
    fireEvent.click(screen.getByRole("checkbox", {
          name: /Send requests without Kassiber's no-retention flag/,
          hidden: true,
        }));
    fireEvent.click(ackBox());
    await clickTest();
    expect(document.querySelector("[data-test-result]")).not.toBeNull();

    choose("OpenRouter");
    expect(keyValue()).toBe("");
    expect(
      (screen.getByRole("radio", { name: "Auto (recommended)", hidden: true }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(
      screen
        .getByRole("checkbox", {
          name: /Send requests without Kassiber's no-retention flag/,
          hidden: true,
        })
        .getAttribute("aria-checked"),
    ).toBe("false");
    expect(document.querySelector("[data-test-result]")).toBeNull();
    expect(field("Default model (optional)").getAttribute("list")).toBeNull();

    // The OpenAI key never goes to OpenRouter.
    fireEvent.click(ackBox());
    mocks.invoke.mockClear();
    await clickTest();
    for (const args of testCalls()) {
      expect(args).not.toHaveProperty("api_key");
      expect(args.base_url).toBe("https://openrouter.ai/api/v1");
    }
  });

  it("clears a typed key when the address moves to another origin", async () => {
    mount();
    choose("OpenRouter");
    typeKey("sk-or-secret");
    type("Base URL or CLI locator", "https://evil.example.test/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(keyValue()).toBe("");
    expect(document.querySelector("[data-key-cleared]")?.textContent).toBe(CLEARED);

    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()).toHaveLength(1);
    expect(testCalls()[0]).not.toHaveProperty("api_key");
  });

  it("keeps the key for another path on the same origin", async () => {
    mount();
    choose("OpenRouter");
    typeKey("sk-or-secret");
    type("Base URL or CLI locator", "https://OpenRouter.ai/api/v2");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(keyValue()).toBe("sk-or-secret");
    expect(document.querySelector("[data-key-cleared]")).toBeNull();
    fireEvent.click(ackBox());
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ api_key: "sk-or-secret" });
  });

  it("sends no key to a new origin, even straight from the field", async () => {
    mount();
    choose("OpenRouter");
    fireEvent.click(ackBox());
    typeKey("sk-or-secret");
    // Retargeted and tested without leaving the field.
    type("Base URL or CLI locator", "https://evil.example.test/v1");
    await clickTest();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(document.querySelector("[data-key-cleared]")?.textContent).toBe(CLEARED);

    // Saving refuses too, instead of storing the key with the new address:
    // the wizard stays on the connection, which says why.
    typeKey("sk-other");
    type("Base URL or CLI locator", "https://third.example.test/v1");
    await submit();
    expect(mocks.mutations["ai.providers.create"]).not.toHaveBeenCalled();
    expect(mocks.mutations["ai.providers.set_api_key"]).not.toHaveBeenCalled();
    expect(stepInView()).toBe("connect");
    expect(keyValue()).toBe("");
    expect(document.querySelector("[data-key-cleared]")?.textContent).toBe(CLEARED);
  });

  it("binds a key typed before the address to the first address it is used with", async () => {
    startCustom();
    type("Name", "Gateway");
    typeKey("sk-early");
    type("Base URL or CLI locator", "http://127.0.0.1:8080/v1");
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ api_key: "sk-early" });
    type("Base URL or CLI locator", "http://127.0.0.1:9090/v1");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(keyValue()).toBe("");
  });
});

describe("AiProviderForm choices belong to their endpoint", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(testAnswer({}));
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const OMIT_NAME = /Send requests without Kassiber's no-retention flag/;
  const RESET = "No-retention exemption reset for the new address.";
  const saved: ExistingAiProvider = {
    name: "gw",
    base_url: "https://llm.example.test/v1",
    kind: "remote",
    has_api_key: true,
    acknowledged_at: "2026-10-01T00:00:00Z",
    api_format: "chat_completions",
    detected_api_format: "chat_completions",
    omit_store_field: true,
  };

  function omitChecked(): string | null {
    return screen.getByRole("checkbox", { name: OMIT_NAME, hidden: true }).getAttribute("aria-checked");
  }

  it("resets the exemption and the format when the address changes, and says so", async () => {
    mount(saved);
    expect(omitChecked()).toBe("true");
    // Same origin, another path: still another endpoint.
    type("Base URL or CLI locator", "https://llm.example.test/v2");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(omitChecked()).toBe("false");
    expect(
      (screen.getByRole("radio", { name: "Auto (recommended)", hidden: true }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(document.querySelector("[data-endpoint-settings-reset]")?.textContent).toBe(RESET);

    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).toMatchObject({
      base_url: "https://llm.example.test/v2",
      omit_store_field: false,
      api_format: "auto",
    });
  });

  it("does not test or save the old choice straight from the field", async () => {
    mount(saved);
    type("Base URL or CLI locator", "https://llm.example.test/v2");
    await clickTest();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(document.querySelector("[data-endpoint-settings-reset]")?.textContent).toBe(RESET);
    await clickTest();
    expect(testCalls()[0]).toMatchObject({ omit_store_field: false, api_format: "auto" });

    cleanup();
    mocks.mutations = {};
    mount(saved);
    type("Base URL or CLI locator", "https://llm.example.test/v2");
    await submit();
    expect(mocks.mutations["ai.providers.update"]).not.toHaveBeenCalled();
    expect(
      screen.getByText("Settings chosen for the old address were reset; review them and save again."),
    ).toBeTruthy();
  });

  it("clears the old address's test result and keeps a choice made for the new one", async () => {
    mocks.invoke.mockResolvedValue(testAnswer({ detected_api_format: "chat_completions" }));
    mount({ ...saved, omit_store_field: false, api_format: "auto" });
    await clickTest();
    expect(document.querySelector("[data-test-result]")).not.toBeNull();
    type("Base URL or CLI locator", "https://llm.example.test/v2");
    // Choosing for the new address, without leaving the field first.
    fireEvent.click(screen.getByRole("checkbox", { name: OMIT_NAME, hidden: true }));
    expect(document.querySelector("[data-test-result]")).toBeNull();
    expect(omitChecked()).toBe("true");
    await clickTest();
    expect(testCalls()[1]).toMatchObject({ omit_store_field: true });
  });

  it("keeps the choices while the address stays", async () => {
    mount(saved);
    type("Notes (optional)", "edited");
    type("Base URL or CLI locator", "https://llm.example.test/v1 ");
    fireEvent.blur(field("Base URL or CLI locator"));
    expect(omitChecked()).toBe("true");
    expect(document.querySelector("[data-endpoint-settings-reset]")).toBeNull();
    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).toMatchObject({
      omit_store_field: true,
      api_format: "chat_completions",
    });
  });
});

describe("AiProviderForm wizard", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(testAnswer({}));
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  function trackLabels(): string[] {
    // A done step shows a check mark in place of its number.
    return Array.from(screen.getByRole("navigation").querySelectorAll("li")).map((item) =>
      (item.textContent ?? "").replace(/^\d+/, ""),
    );
  }

  it("walks a local server from provider to model without a privacy step", async () => {
    mount();
    expect(stepInView()).toBe("provider");
    expect(trackLabels()).toEqual(["Provider", "Connection", "Model"]);
    pick("Ollama");
    next();
    expect(stepInView()).toBe("connect");
    expect(field("Name").value).toBe("Ollama");
    next();
    expect(stepInView()).toBe("check");
    // Nothing was contacted on the way.
    expect(mocks.invoke).not.toHaveBeenCalled();

    // Back keeps what was entered; the track jumps back to a done step.
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(stepInView()).toBe("connect");
    next();
    fireEvent.click(screen.getByRole("button", { name: "Provider: Ollama" }));
    expect(stepInView()).toBe("provider");
    expect(screen.getByRole("radio", { name: "Ollama" }).getAttribute("aria-checked")).toBe("true");
  });

  it("asks on its own step before prompts leave this device", () => {
    mount();
    pick("Claude CLI");
    walkTo("check");
    expect(stepInView()).toBe("privacy");
    expect(trackLabels()).toEqual(["Provider", "Connection", "Privacy", "Model"]);
    const privacy = document.querySelector('[data-wizard-step="privacy"]')!;
    expect(privacy.textContent).toContain("Prompts to Claude CLI leave this computer");
    // An app speaks with its own settings; an HTTP API is asked not to keep requests.
    expect(privacy.textContent).toContain("its vendor's terms apply");
    expect(privacy.textContent).not.toContain("store: false");
    cleanup();

    mount();
    pick("OpenAI");
    type("API key (required)", "sk-test");
    walkTo("check");
    expect(stepInView()).toBe("privacy");
    expect(document.querySelector('[data-wizard-step="privacy"]')!.textContent).toContain(
      "store: false",
    );
  });

  it("opens an existing provider at its connection, saveable from any step", async () => {
    mount({
      name: "box",
      display_name: "Box",
      base_url: "http://127.0.0.1:1234/v1",
      kind: "local",
      has_api_key: false,
    });
    expect(stepInView()).toBe("connect");
    expect(trackLabels()).toEqual(["Connection", "Model"]);
    expect(screen.queryByRole("radio", { name: /^On this computer/ })).toBeNull();
    type("Name", "Workstation");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });
    expect(mocks.mutations["ai.providers.update"]).toHaveBeenCalledWith(
      expect.objectContaining({ name: "box", display_name: "Workstation" }),
    );
  });

  it("needs a name before moving on", () => {
    startCustom();
    type("Base URL or CLI locator", "http://127.0.0.1:1234/v1");
    walkTo("check");
    expect(stepInView()).toBe("connect");
    expect(errorText()).toBe("Give this provider a name.");
  });
});

describe("AiProviderForm agent access", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.mutations = {};
  });
  afterEach(() => {
    cleanup();
  });

  const RESTRICT_NAME = /Restrict what the assistant can use/;

  const saved: ExistingAiProvider = {
    name: "gateway",
    base_url: "https://llm.example.test/v1",
    kind: "remote",
    has_api_key: true,
    acknowledged_at: "2026-10-01T00:00:00Z",
  };

  it("saves full access unless the user restricts the provider", async () => {
    mount(saved);
    walkTo("privacy");
    const box = screen.getByRole("checkbox", { name: RESTRICT_NAME });
    expect(box.getAttribute("aria-checked")).toBe("false");
    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).toMatchObject({
      agent_access: "full",
    });
  });

  it("keeps a saved restriction and saves the user's change", async () => {
    mount({ ...saved, agent_access: "restricted" });
    walkTo("privacy");
    const box = screen.getByRole("checkbox", { name: RESTRICT_NAME });
    expect(box.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(box);
    await submit();
    expect(mocks.mutations["ai.providers.update"].mock.calls[0][0]).toMatchObject({
      agent_access: "full",
    });
  });
});
