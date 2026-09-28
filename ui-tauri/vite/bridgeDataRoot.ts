/** Read the preview's launch book from `KASSIBER_DEV_DATA_ROOT`, if set. */
export function devDataRootSeed(
  env: NodeJS.ProcessEnv = process.env,
): string | null {
  return env.KASSIBER_DEV_DATA_ROOT?.trim() || null;
}

// Which book the dev bridge's daemon opens. The seed is the preview's launch
// book (`pnpm dev:demo`, a preview launcher's `--book`). Clearing an imported
// project returns to that seed; only an unseeded bridge falls back to the
// daemon's implicit default root, which on a developer machine is their real
// book.
export class BridgeDataRoot {
  readonly seed: string | null;
  private current: string | null;
  private readonly onChange: () => void;

  /** `onChange` runs whenever the root changes, so the daemon can restart. */
  constructor(seed: string | null, onChange: () => void) {
    this.seed = seed;
    this.current = seed;
    this.onChange = onChange;
  }

  get(): string | null {
    return this.current;
  }

  set(dataRoot: string | null) {
    if (this.current === dataRoot) return;
    this.current = dataRoot;
    this.onChange();
  }

  /** Return to the launch book rather than the implicit default root. */
  reset() {
    this.set(this.seed);
  }

  /** Arguments that start `kassiber daemon` against the current root. */
  daemonArgs(runnerArgs: readonly string[]): string[] {
    const args = [...runnerArgs];
    if (this.current) {
      args.push("--data-root", this.current);
    }
    args.push("daemon");
    return args;
  }
}
