# Desktop Reference

Kassiber's desktop shell uses Tauri 2 + React + TypeScript with the Python
core running as a long-lived sidecar daemon over JSONL. See
[../plan/01-stack-decision.md](../plan/01-stack-decision.md) for the stack
decision and [../plan/04-desktop-ui.md](../plan/04-desktop-ui.md) for the
implementation plan.

The desktop shell is a pre-alpha preview. It already uses real daemon-backed
paths for the main setup, review, report, export, assistant, and diagnostics
workflows, but the CLI is still the most complete and scriptable control
surface. See [../quickstart.md](../quickstart.md) for the end-to-end
workflows and [machine-output.md](machine-output.md) for the JSON envelope
contract the shell consumes through the daemon.

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

For a real installable `.app` built locally on Apple Silicon (without
Rosetta), use `./scripts/build-macos-arm64-app.sh`. The build is unsigned
and ad-hoc signed; first-launch Gatekeeper handling is documented in
[prerelease-binaries.md → Local Apple Silicon build](prerelease-binaries.md#local-apple-silicon-build).

The authenticated shell includes a Diagnostics screen with a redacted
daemon/transport activity log and a downloadable JSON export. It is meant for
prerelease and development troubleshooting: request logs include argument keys,
not argument values, while terminal daemon errors keep their structured
message, hint, and redacted details when the daemon exposes them.

The Connections detail page includes a read-only Coins table for chain-backed
wallet sources. Refreshing a descriptor/xpub/address wallet updates the local
output inventory, and the detail page shows current unspent outputs with
outpoint, amount, confirmation state, receive/change branch/index when known,
address or safe label, and source freshness. The table is inventory-only:
there is no spend, PSBT, signing, broadcast, coin-selection, or freeze action.
Unsupported file/BTCPay/Lightning-style sources show an unsupported state, and
Liquid sources show an unblind blocker unless Kassiber has descriptor material
that can unblind outputs locally.

Settings -> AI providers displays each provider's API-key presence plus storage
location/state. Saving provider metadata does not include the raw key in the
create/update request; when the API-key field is filled, the form sends the
narrow `ai.providers.set_api_key` daemon kind and receives only redacted
metadata back. Connection tests use the stored provider key; a newly typed key
must be saved through `ai.providers.set_api_key` before testing. Settings can
move an existing provider key between `sqlcipher_inline` and the native store
with `ai.providers.move_api_key`. macOS production-signed builds may default to
Keychain; unsigned/ad-hoc/unknown macOS builds keep Keychain opt-in
experimental copy because rebuilds or app identity changes can prompt again.
Windows uses user-scope Credential Manager/DPAPI when available. Linux falls
back explicitly to `sqlcipher_inline` when no desktop secret service, D-Bus
session, or unlocked collection is available. There is no
plaintext fallback and no remember-unlock behavior.

Settings -> Desktop -> Terminal command can install a user-local `kassiber`
launcher without administrator privileges. It writes a small managed launcher
under the user's bin directory (for example `~/.local/bin/kassiber`) that
forwards to the installed desktop executable and its bundled CLI sidecar. If
that directory is not on PATH, Settings shows the PATH line to add to the
user's shell.

Package managers can link the bundled launcher at
`Kassiber.app/Contents/Resources/bin/kassiber` instead. The first target is a
project-owned Homebrew tap; see [Homebrew Cask](homebrew-cask.md).

The GUI executable also works as a CLI forwarder when launched directly with
`--cli ...`. Examples:

```bash
Kassiber.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

If the app executable is symlinked with the exact executable stem `kassiber`,
plain CLI args are also forwarded, but the Settings launcher is preferred
because it explicitly passes `--cli`:

```bash
ln -s /Applications/Kassiber.app/Contents/MacOS/kassiber-ui /usr/local/bin/kassiber
kassiber status
```

Use `--cli ...` for any other symlink or executable name.
