# Desktop Reference

Kassiber's desktop shell uses Tauri 2 + React + TypeScript with the Python
core running as a long-lived sidecar daemon over JSONL. See
[../plan/01-stack-decision.md](../plan/01-stack-decision.md) for the stack
decision and [../plan/04-desktop-ui.md](../plan/04-desktop-ui.md) for the
implementation plan.

The desktop shell is a pre-alpha preview. It already uses real daemon-backed
paths for the main setup, review, report, export, assistant, and diagnostics
workflows, but the CLI is still the most complete and scriptable control
surface. See [../../README.md](../../README.md) for the quick start and
[machine-output.md](machine-output.md) for the JSON envelope contract the shell
consumes through the daemon.

Current development modes:

- `pnpm dev` in `ui-tauri/` runs the browser dashboard against the
  loopback-only Vite daemon bridge by default. `pnpm dev:bridge` is the
  explicit form of the same mode. Use `pnpm dev:browser` for mock daemon
  fixtures when you want disconnected UI layout work.
- `pnpm tauri:dev` runs the Tauri shell, starts `python -m kassiber daemon`,
  and calls the Rust `daemon_invoke` boundary. The command allowlists the
  current UI data, export, and action kinds. Report exports write under the
  managed `exports/reports/` state directory, and the desktop shell exposes a
  narrow `open_exported_file` command that opens completed PDF/XLSX/CSV report
  files with the system default app. Transaction explorer links use a separate
  `open_external_url` command that only accepts absolute HTTP/HTTPS URLs with a
  host and no embedded credentials before handing them to the system default
  browser.
  The Welcome screen can also open existing local books: the native
  folder picker opens at `~/.kassiber`, accepts either the state root or the
  `data/` folder, restarts the sidecar daemon with that `--data-root`, and then
  lists local books grouped by books set. SQLCipher databases must be unlocked
  before book names can be read because the internal workspace/profile rows live
  inside the encrypted database. macOS uses the system folder picker, Windows
  uses the system folder dialog through PowerShell/.NET, and Linux desktops use
  `zenity`, `kdialog`, or `yad` when one of those pickers is installed.
  The supervisor uses `.venv/bin/python` when present, then `python3`, unless
  `KASSIBER_PYTHON` is set. `KASSIBER_REPO_ROOT` can point a dev shell at a
  different checkout.

Current prerelease desktop packages bundle a one-file `kassiber-cli-*`
sidecar built with PyInstaller. At runtime the supervisor prefers
`KASSIBER_PYTHON` when it is explicitly set, then the bundled sidecar from the
app resources, then the development Python fallback above. The same
`KASSIBER_PYTHON` override applies to installed-app CLI forwarding.

The authenticated shell includes a Diagnostics screen with a redacted
daemon/transport activity log and a downloadable JSON export. It is meant for
prerelease and development troubleshooting: request logs include argument keys,
not argument values, while terminal daemon errors keep their structured
message, hint, and redacted details when the daemon exposes them.

The GUI executable also works as a CLI forwarder when launched with
`--cli ...`. Examples:

```bash
Kassiber.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

If the app executable is symlinked with the exact executable stem `kassiber`,
plain CLI args are also forwarded:

```bash
ln -s /Applications/Kassiber.app/Contents/MacOS/kassiber-ui /usr/local/bin/kassiber
kassiber status
```

Use `--cli ...` for any other symlink or executable name.
