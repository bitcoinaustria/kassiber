import {
  emitAppLog,
  type AppLogField,
  type AppLogLevel,
} from "./appLogs";

const STACK_HEAD_LINES = 10;
const THROTTLE_KEY_CHARS = 200;
const THROTTLE_WINDOW_MS = 10_000;
const THROTTLE_MAX_PER_WINDOW = 5;

interface CaptureRecord {
  level: AppLogLevel;
  module: string;
  file: string;
  line: number;
  msg: string;
  fields: Record<string, AppLogField>;
}

interface ThrottleWindow {
  key: string;
  level: AppLogLevel;
  module: string;
  startedAt: number;
  emitted: number;
  suppressed: number;
}

let installed = false;
let capturing = false;
let now: () => number = () => Date.now();
let throttle: ThrottleWindow | null = null;
let originalConsoleError: typeof console.error | null = null;
let originalConsoleWarn: typeof console.warn | null = null;
let onWindowError: ((event: ErrorEvent) => void) | null = null;
let onUnhandledRejection: ((event: PromiseRejectionEvent) => void) | null =
  null;

export function installGlobalErrorCapture(options?: {
  now?: () => number;
}): void {
  if (installed) return;
  installed = true;
  now = options?.now ?? (() => Date.now());

  onWindowError = (event) => {
    capture({
      level: "error",
      module: "window",
      file: basename(event.filename ?? ""),
      line: event.lineno ?? 0,
      msg: event.message || "Uncaught error",
      fields: stackFields(event.error),
    });
  };
  onUnhandledRejection = (event) => {
    capture({
      level: "error",
      module: "window",
      file: "",
      line: 0,
      msg: `Unhandled promise rejection: ${formatConsoleArg(event.reason)}`,
      fields: stackFields(event.reason),
    });
  };
  window.addEventListener("error", onWindowError);
  window.addEventListener("unhandledrejection", onUnhandledRejection);

  originalConsoleError = console.error;
  originalConsoleWarn = console.warn;
  const passthroughError = originalConsoleError.bind(console);
  const passthroughWarn = originalConsoleWarn.bind(console);
  console.error = (...args: unknown[]) => {
    passthroughError(...args);
    captureConsole("error", args);
  };
  console.warn = (...args: unknown[]) => {
    passthroughWarn(...args);
    captureConsole("warning", args);
  };
}

export function uninstallGlobalErrorCapture(): void {
  if (!installed) return;
  installed = false;
  if (originalConsoleError) console.error = originalConsoleError;
  if (originalConsoleWarn) console.warn = originalConsoleWarn;
  if (onWindowError) window.removeEventListener("error", onWindowError);
  if (onUnhandledRejection) {
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
  }
  originalConsoleError = null;
  originalConsoleWarn = null;
  onWindowError = null;
  onUnhandledRejection = null;
  throttle = null;
  capturing = false;
  now = () => Date.now();
}

export function stackHead(error: unknown): string {
  if (!(error instanceof Error) || !error.stack) return "";
  return error.stack.split("\n").slice(0, STACK_HEAD_LINES).join("\n");
}

function captureConsole(level: AppLogLevel, args: unknown[]): void {
  const error = args.find((arg) => arg instanceof Error);
  capture({
    level,
    module: "console",
    file: "",
    line: 0,
    msg: args.map(formatConsoleArg).join(" ") || "(no message)",
    fields: stackFields(error),
  });
}

function capture(record: CaptureRecord): void {
  // Subscribers may log during notification; never re-enter the ring.
  if (capturing) return;
  capturing = true;
  try {
    admit(record);
  } catch {
    // Capture must never break the original console/error path.
  } finally {
    capturing = false;
  }
}

function admit(record: CaptureRecord): void {
  const key = `${record.level}:${record.msg.slice(0, THROTTLE_KEY_CHARS)}`;
  const at = now();
  if (
    throttle &&
    (throttle.key !== key || at - throttle.startedAt >= THROTTLE_WINDOW_MS)
  ) {
    flushSuppressed();
  }
  if (!throttle) {
    throttle = {
      key,
      level: record.level,
      module: record.module,
      startedAt: at,
      emitted: 0,
      suppressed: 0,
    };
  }
  if (throttle.emitted >= THROTTLE_MAX_PER_WINDOW) {
    throttle.suppressed += 1;
    return;
  }
  throttle.emitted += 1;
  emitAppLog(record);
}

function flushSuppressed(): void {
  const closed = throttle;
  throttle = null;
  if (!closed || closed.suppressed === 0) return;
  emitAppLog({
    level: closed.level,
    module: closed.module,
    file: "",
    line: 0,
    msg: `suppressed ${closed.suppressed} duplicate records`,
    fields: {
      duplicate_msg_head: {
        type: "text",
        value: closed.key.slice(closed.level.length + 1),
      },
    },
  });
}

function stackFields(error: unknown): Record<string, AppLogField> {
  const stack = stackHead(error);
  return stack ? { stack: { type: "text", value: stack } } : {};
}

function formatConsoleArg(arg: unknown): string {
  if (typeof arg === "string") return arg;
  if (arg instanceof Error) return arg.message || String(arg);
  if (
    typeof arg === "number" ||
    typeof arg === "boolean" ||
    arg === null ||
    arg === undefined
  ) {
    return String(arg);
  }
  try {
    return JSON.stringify(arg);
  } catch {
    return String(arg);
  }
}

function basename(filename: string): string {
  if (!filename) return "";
  const cleaned = filename.split(/[?#]/, 1)[0];
  return cleaned.split(/[\\/]/).filter(Boolean).pop() ?? "";
}
