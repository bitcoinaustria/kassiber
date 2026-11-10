"""Run the Linux interop test on an isolated bus and disposable native keyring.

Optional local acceptance helper: requires dbus-run-session and
GNOME Keyring. Run using the locked Python environment from the repository.
No credentials or daemon files use the account's normal home or session bus.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
TEST = "tests/test_rust_store_interop.py::test_python_secret_service_item_rust_reads_updates_in_place"


def child() -> int:
    import secretstorage
    from jeepney.bus_messages import message_bus
    from jeepney.io.blocking import Proxy

    control = Path(os.environ["HOME"]) / "control"
    control.mkdir(mode=0o700)
    daemon = subprocess.Popen(
        ["gnome-keyring-daemon", "--foreground", "--unlock", "--components=secrets", "--control-directory", str(control)],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )
    try:
        daemon.stdin.write(b"Synthetic disposable R1 keyring")
        daemon.stdin.close()
        connection = secretstorage.dbus_init()
        bus = Proxy(message_bus, connection)
        for _ in range(50):
            try:
                # Do not auto-activate a second daemon during startup.
                if not bus.NameHasOwner("org.freedesktop.secrets")[0]:
                    time.sleep(0.1)
                    continue
                collection = secretstorage.get_default_collection(connection)
                if not collection.is_locked():
                    break
            except Exception:
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("disposable Secret Service did not become ready")
        connection.close()
        return subprocess.call([sys.executable, "-m", "pytest", TEST, "-q", "-rs"], cwd=ROOT)
    finally:
        daemon.terminate()
        daemon.wait(timeout=10)


def main() -> int:
    if sys.platform != "linux" or not all(shutil.which(name) for name in ("dbus-run-session", "gnome-keyring-daemon")):
        raise SystemExit("requires Linux, dbus-run-session and gnome-keyring-daemon")
    if sys.argv[1:] == ["--private-child"]:
        return child()
    account_home = Path.home()
    with tempfile.TemporaryDirectory(prefix="kassiber-r1-native-") as temporary:
        root = Path(temporary)
        env = dict(os.environ)
        # Tool installations/caches remain readable while all daemon state is
        # redirected. Cargo's test build is locked and offline.
        env.setdefault("MISE_DATA_DIR", str(account_home / ".local/share/mise"))
        env.setdefault("CARGO_HOME", str(account_home / ".cargo"))
        env.update(HOME=str(root), XDG_DATA_HOME=str(root / "data"),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
                   XDG_RUNTIME_DIR=str(root / "runtime"))
        (root / "runtime").mkdir(mode=0o700)
        return subprocess.call(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()), "--private-child"], cwd=ROOT, env=env)


if __name__ == "__main__":
    raise SystemExit(main())
