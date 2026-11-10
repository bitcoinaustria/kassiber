"""A packaged Kassiber binary opens no network socket on launch, status, or idle.

The production binary has no test egress observer (an armed
`KASSIBER_TEST_NO_EGRESS` makes it exit 78), so this check watches it from the
operating system instead. Every child runs under a seccomp filter that kills
the process on any `socket(AF_INET | AF_INET6 | AF_PACKET)` call, loopback
included, and on `io_uring_setup`; an `LD_PRELOAD` shim records every libc
resolver call (`getaddrinfo`, `gethostbyname*`, `getnameinfo`), because glibc
can resolve over systemd-resolved's Unix socket without opening an IP socket.

Product egress stays enabled and the book carries saved backends (an
unresolvable `.invalid` Esplora and a loopback Electrum), so nothing but the
product's own consent rules keeps it quiet: a saved backend is configuration,
not consent. The scenarios are launch (`--version`, `--help`), status on a
fresh, a plaintext and a SQLCipher book, and a daemon that answers display
reads and then sits idle.

Point `KASSIBER_PACKAGED_CLI` at the executable to check (for example the
`kassiber` extracted from a release archive). Linux x86-64 and AArch64 only;
the shim needs a C compiler. Runs under plain `python3 -m unittest`, so
release CI needs no project environment for it.
"""
from __future__ import annotations

import ctypes
import json
import os
import platform
import queue
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

PACKAGED_CLI = os.environ.get("KASSIBER_PACKAGED_CLI", "")
IDLE_SECONDS = float(os.environ.get("KASSIBER_PACKAGED_IDLE_SECONDS", "8"))
PASSPHRASE = "packaged no-egress passphrase"
DESCRIPTOR = (
    "wpkh([91790c21/84h/0h/0h]xpub6CpT4xRJntYQuFwkiZfSiK7jEtPM3e9nUfJKQ8ATWkgYJui4WS5HmzjD6C7"
    "avtwAFtmEyAusdFFxg2rZMs73SoSVkPQfEaQtrLGox92JNWn/0/*)"
)

# seccomp_data: nr at 0, arch at 4, args[0] (low word on little endian) at 16.
_ARCHES = {
    "x86_64": {"audit": 0xC000003E, "socket": 41, "io_uring_setup": 425, "x32": True},
    "aarch64": {"audit": 0xC00000B7, "socket": 198, "io_uring_setup": 425, "x32": False},
}
_AF_INET, _AF_INET6, _AF_PACKET = 2, 10, 17
_SECCOMP_RET_ALLOW = 0x7FFF0000
_SECCOMP_RET_KILL_PROCESS = 0x80000000
_PR_SET_NO_NEW_PRIVS = 38
_SECCOMP_SET_MODE_FILTER = 1
_SYS_SECCOMP = {"x86_64": 317, "aarch64": 277}

_RESOLVER_SHIM = r"""
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <unistd.h>

static void note(const char *what, const char *name) {
    const char *path = getenv("KASSIBER_RESOLVER_SHIM_LOG");
    if (!path) return;
    int fd = open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
    if (fd < 0) return;
    char line[512];
    int n = snprintf(line, sizeof line, "%d %s %s\n", (int)getpid(), what, name ? name : "-");
    if (n > 0) {
        if (n >= (int)sizeof line) n = (int)sizeof line - 1;
        ssize_t ignored = write(fd, line, (size_t)n);
        (void)ignored;
    }
    close(fd);
}

int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints,
                struct addrinfo **res) {
    (void)service; (void)hints; (void)res;
    note("getaddrinfo", node);
    return EAI_FAIL;
}

int getnameinfo(const struct sockaddr *addr, socklen_t addrlen, char *host, socklen_t hostlen,
                char *serv, socklen_t servlen, int flags) {
    (void)addr; (void)addrlen; (void)host; (void)hostlen; (void)serv; (void)servlen; (void)flags;
    note("getnameinfo", NULL);
    return EAI_FAIL;
}

struct hostent *gethostbyname(const char *name) {
    note("gethostbyname", name);
    h_errno = NO_RECOVERY;
    return NULL;
}

struct hostent *gethostbyname2(const char *name, int af) {
    (void)af;
    note("gethostbyname2", name);
    h_errno = NO_RECOVERY;
    return NULL;
}

int gethostbyname_r(const char *name, struct hostent *ret, char *buf, size_t buflen,
                    struct hostent **result, int *h_errnop) {
    (void)ret; (void)buf; (void)buflen;
    note("gethostbyname_r", name);
    *result = NULL;
    *h_errnop = NO_RECOVERY;
    return ENOENT;
}

int gethostbyname2_r(const char *name, int af, struct hostent *ret, char *buf, size_t buflen,
                     struct hostent **result, int *h_errnop) {
    (void)af; (void)ret; (void)buf; (void)buflen;
    note("gethostbyname2_r", name);
    *result = NULL;
    *h_errnop = NO_RECOVERY;
    return ENOENT;
}
"""


def _seccomp_program(machine: str) -> bytes:
    """A classic BPF filter killing the process on IP sockets and io_uring."""
    arch = _ARCHES[machine]

    def stmt(code, k):
        return (code, 0, 0, k)

    def jump(code, k, jt, jf):
        return (code, jt, jf, k)

    LD_W_ABS, JEQ_K, JGE_K, RET_K = 0x20, 0x15, 0x35, 0x06
    KILL, ALLOW = _SECCOMP_RET_KILL_PROCESS, _SECCOMP_RET_ALLOW
    program = [
        stmt(LD_W_ABS, 4),                                  # 0: arch
        jump(JEQ_K, arch["audit"], 1, 0),                   # 1
        stmt(RET_K, KILL),                                  # 2: foreign ABI
        stmt(LD_W_ABS, 0),                                  # 3: syscall nr
    ]
    if arch["x32"]:
        program += [
            jump(JGE_K, 0x40000000, 0, 1),                  # x32 syscall numbers
            stmt(RET_K, KILL),
        ]
    program += [
        jump(JEQ_K, arch["io_uring_setup"], 0, 1),
        stmt(RET_K, KILL),
        jump(JEQ_K, arch["socket"], 1, 0),
        stmt(RET_K, ALLOW),
        stmt(LD_W_ABS, 16),                                 # socket(domain, ...)
        jump(JEQ_K, _AF_INET, 3, 0),
        jump(JEQ_K, _AF_INET6, 2, 0),
        jump(JEQ_K, _AF_PACKET, 1, 0),
        stmt(RET_K, ALLOW),
        stmt(RET_K, KILL),
    ]
    raw = bytearray()
    for code, jt, jf, k in program:
        raw += code.to_bytes(2, "little") + bytes((jt, jf)) + k.to_bytes(4, "little")
    return bytes(raw)


def _install_filter(machine: str):
    """A preexec_fn: no new privileges, then the seccomp filter, then exec."""
    program = _seccomp_program(machine)
    count = len(program) // 8
    libc = ctypes.CDLL(None, use_errno=True)
    filter_buffer = ctypes.create_string_buffer(program, len(program))

    class SockFprog(ctypes.Structure):
        _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.c_void_p)]

    fprog = SockFprog(count, ctypes.cast(filter_buffer, ctypes.c_void_p))
    syscall_number = _SYS_SECCOMP[machine]

    def preexec():
        if libc.prctl(_PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0:
            os._exit(121)
        if libc.syscall(syscall_number, _SECCOMP_SET_MODE_FILTER, 0, ctypes.byref(fprog)) != 0:
            os._exit(122)

    # Keep the buffers alive for as long as the closure is.
    preexec.keep = (filter_buffer, fprog)
    return preexec


def _skip_reason() -> str | None:
    if not PACKAGED_CLI:
        return "set KASSIBER_PACKAGED_CLI to the packaged kassiber executable"
    if sys.platform != "linux" or platform.machine() not in _ARCHES:
        return "the seccomp watcher supports Linux x86-64 and AArch64 only"
    if not (shutil.which("cc") or shutil.which("gcc")):
        return "a C compiler is needed for the resolver shim"
    return None


@unittest.skipIf(_skip_reason(), _skip_reason() or "")
class PackagedCliNoEgressTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.machine = platform.machine()
        cls.cli = str(Path(PACKAGED_CLI).resolve())
        # Not tempfile's default: test-tmp roots put scratch on the build disk.
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-packaged-no-egress-")
        cls.tmp = Path(os.path.realpath(cls._tmp.name))
        compiler = shutil.which("cc") or shutil.which("gcc")
        source = cls.tmp / "resolver_shim.c"
        source.write_text(_RESOLVER_SHIM, encoding="utf-8")
        cls.shim = cls.tmp / "resolver_shim.so"
        subprocess.run(
            [compiler, "-shared", "-fPIC", "-O2", "-o", str(cls.shim), str(source)],
            check=True,
        )
        # A plain function stored on the class would bind as a method.
        cls.preexec = staticmethod(_install_filter(cls.machine))

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def setUp(self):
        self.case = Path(tempfile.mkdtemp(dir=self.tmp, prefix=f"{self._testMethodName}-"))
        self.data_root = self.case / "data"
        self.resolver_log = self.case / "resolver.log"
        prefs = self.case / "preferences"
        prefs.mkdir()
        env = {
            key: value
            for key, value in os.environ.items()
            # The production binary refuses an armed test guard (exit 78);
            # the product switch would hide exactly what this test watches.
            if key not in {"KASSIBER_TEST_NO_EGRESS", "KASSIBER_NO_EGRESS", "LD_PRELOAD"}
            and not key.startswith(("KASSIBER_BACKEND_", "SATBOOKS_BACKEND_"))
        }
        env.update(
            {
                "XDG_DATA_HOME": str(self.case / "xdg-data"),
                "KASSIBER_OFFLINE_PREFERENCE_FILE": str(prefs / "offline-mode.json"),
                "KASSIBER_AGENT_ACCESS_FILE": str(prefs / "agent-access.json"),
                "KASSIBER_UPDATE_PREFERENCE_FILE": str(prefs / "update-check.json"),
                "KASSIBER_UPDATE_CACHE_FILE": str(prefs / "update-cache.json"),
                "LD_PRELOAD": str(self.shim),
                "KASSIBER_RESOLVER_SHIM_LOG": str(self.resolver_log),
                "TMPDIR": str(self.case),
            }
        )
        self.env = env

    # -- helpers ---------------------------------------------------------

    def watched(self, argv, *, stdin_text=None, timeout=300):
        completed = subprocess.run(
            argv,
            input=stdin_text,
            capture_output=True,
            text=True,
            env=self.env,
            cwd=self.case,
            preexec_fn=self.preexec,
            timeout=timeout,
        )
        self.assert_not_killed(completed.returncode, argv, completed.stderr)
        return completed

    def kassiber(self, *args, stdin_text=None, check=True):
        argv = [self.cli, "--data-root", str(self.data_root), "--machine", *args]
        completed = self.watched(argv, stdin_text=stdin_text)
        if check and completed.returncode != 0:
            self.fail(f"{args} exited {completed.returncode}: {completed.stdout}{completed.stderr}")
        return completed

    def assert_not_killed(self, returncode, argv, stderr=""):
        self.assertNotIn(returncode, (121, 122), "the seccomp filter could not be installed")
        self.assertNotEqual(
            returncode,
            -signal.SIGSYS,
            f"{argv[1:]} opened an IP socket (killed by the seccomp watcher). stderr: {stderr[-2000:]}",
        )

    def assert_no_resolver_calls(self):
        calls = self.resolver_log.read_text(encoding="utf-8") if self.resolver_log.exists() else ""
        self.assertEqual(calls, "", f"resolver calls recorded:\n{calls}")

    def seed_book_with_backends(self):
        self.kassiber("init")
        self.kassiber("workspaces", "create", "Smoke")
        self.kassiber("profiles", "create", "--workspace", "Smoke", "--fiat-currency", "EUR",
                      "--tax-country", "generic", "Main")
        self.kassiber("backends", "create", "--kind", "esplora",
                      "--url", "https://egress-probe.invalid/api", "dnsesplora")
        self.kassiber("backends", "create", "--kind", "electrum",
                      "--url", "tcp://127.0.0.1:9", "loopelectrum")
        self.kassiber("backends", "set-default", "dnsesplora")
        self.kassiber("wallets", "create", "--workspace", "Smoke", "--profile", "Main",
                      "--label", "Watched", "--kind", "address", "--backend", "dnsesplora",
                      "--address", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080")

    # -- controls --------------------------------------------------------

    def test_watchers_catch_a_socket_and_a_lookup(self):
        """Both watchers fire on a deliberate attempt, so silence means something."""
        socket_attempt = subprocess.run(
            [sys.executable, "-I", "-c", "import socket; socket.socket()"],
            env=self.env, cwd=self.case, preexec_fn=self.preexec, capture_output=True,
        )
        self.assertEqual(socket_attempt.returncode, -signal.SIGSYS)
        loopback_attempt = subprocess.run(
            [sys.executable, "-I", "-c",
             "import socket; socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)"],
            env=self.env, cwd=self.case, preexec_fn=self.preexec, capture_output=True,
        )
        self.assertEqual(loopback_attempt.returncode, -signal.SIGSYS)
        unix_socket = subprocess.run(
            [sys.executable, "-I", "-c", "import socket; socket.socket(socket.AF_UNIX)"],
            env=self.env, cwd=self.case, preexec_fn=self.preexec, capture_output=True,
        )
        self.assertEqual(unix_socket.returncode, 0, unix_socket.stderr)
        lookup = subprocess.run(
            [sys.executable, "-I", "-c",
             "import socket\ntry:\n socket.getaddrinfo('egress-probe.invalid', 443)\nexcept OSError:\n pass"],
            env=self.env, cwd=self.case, capture_output=True,
        )
        self.assertEqual(lookup.returncode, 0, lookup.stderr)
        self.assertIn("getaddrinfo egress-probe.invalid", self.resolver_log.read_text(encoding="utf-8"))

    def test_watcher_stops_a_user_triggered_sync_of_the_packaged_binary(self):
        """The binary's own socket path trips the watcher before it connects."""
        self.kassiber("init")
        self.kassiber("workspaces", "create", "Smoke")
        self.kassiber("profiles", "create", "--workspace", "Smoke", "--fiat-currency", "EUR",
                      "--tax-country", "generic", "Main")
        self.kassiber("backends", "create", "--kind", "esplora",
                      "--url", "http://127.0.0.1:9/api", "loopesplora")
        self.kassiber("wallets", "create", "--workspace", "Smoke", "--profile", "Main",
                      "--label", "Loopback", "--kind", "descriptor", "--backend", "loopesplora",
                      "--descriptor-stdin", "--gap-limit", "1", stdin_text=DESCRIPTOR + "\n")
        sync = subprocess.run(
            [self.cli, "--data-root", str(self.data_root), "--machine", "wallets", "sync",
             "--workspace", "Smoke", "--profile", "Main", "--wallet", "Loopback"],
            capture_output=True, text=True, env=self.env, cwd=self.case,
            preexec_fn=self.preexec, timeout=300,
        )
        self.assertEqual(sync.returncode, -signal.SIGSYS, sync.stdout + sync.stderr)

    # -- the packaged binary ---------------------------------------------

    def test_launch_and_fresh_status_open_no_socket(self):
        self.watched([self.cli, "--version"])
        self.watched([self.cli, "--help"])
        fresh = self.kassiber("status", check=False)
        self.assertEqual(json.loads(fresh.stdout)["error"]["code"], "not_initialized")
        self.assert_no_resolver_calls()

    def test_status_with_saved_backends_opens_no_socket(self):
        self.seed_book_with_backends()
        status = self.kassiber("status")
        self.assertEqual(json.loads(status.stdout)["kind"], "status")
        self.kassiber("backends", "list")
        self.kassiber("wallets", "list", "--workspace", "Smoke", "--profile", "Main")
        self.assert_no_resolver_calls()

    def test_encrypted_book_status_opens_no_socket(self):
        self.seed_book_with_backends()
        self.kassiber("secrets", "init", "--new-passphrase-fd", "0", stdin_text=PASSPHRASE + "\n")
        header = (self.data_root / "kassiber.sqlite3").read_bytes()[:16]
        self.assertNotEqual(header, b"SQLite format 3\x00", "secrets init left a plaintext book")
        status = self.kassiber("--db-passphrase-fd", "0", "status", stdin_text=PASSPHRASE + "\n")
        self.assertEqual(json.loads(status.stdout)["kind"], "status")
        self.assert_no_resolver_calls()

    def test_daemon_display_reads_and_idle_open_no_socket(self):
        self.seed_book_with_backends()
        process = subprocess.Popen(
            [self.cli, "--data-root", str(self.data_root), "daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.env,
            cwd=self.case,
            preexec_fn=self.preexec,
        )
        lines: queue.Queue = queue.Queue()
        reader = threading.Thread(
            target=lambda: [lines.put(line) for line in process.stdout], daemon=True
        )
        reader.start()

        def next_message(timeout=120):
            try:
                return json.loads(lines.get(timeout=timeout))
            except queue.Empty:
                process.kill()
                process.wait()
                self.assert_not_killed(process.returncode, ["daemon"], process.stderr.read())
                self.fail("the daemon stopped answering")

        def send(request):
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()

        try:
            ready = next_message()
            self.assertEqual(ready["kind"], "daemon.ready")
            supported = set(ready.get("supported_kinds") or ready.get("data", {}).get("supported_kinds", []))
            reads = [
                kind
                for kind in (
                    "status",
                    "ui.overview.snapshot",
                    "ui.workspace.overview.snapshot",
                    "ui.wallets.list",
                    "ui.backends.list",
                    "ui.backends.settings.list",
                    "ui.maintenance.settings",
                    "ui.freshness.status",
                    "ui.agent_access.status",
                )
                if kind in supported
            ]
            self.assertIn("status", reads)
            for index, kind in enumerate(reads):
                send({"request_id": f"read-{index}", "kind": kind, "args": {}})
                answer = next_message()
                self.assertEqual(answer.get("request_id"), f"read-{index}", answer)
            # Idle: no timer, freshness policy or update check may reach out.
            time.sleep(IDLE_SECONDS)
            self.assertIsNone(process.poll(), "the daemon exited while idle")
            send({"request_id": "after-idle", "kind": "status", "args": {}})
            self.assertEqual(next_message().get("request_id"), "after-idle")
            send({"request_id": "shutdown", "kind": "daemon.shutdown"})
            self.assertEqual(next_message()["kind"], "daemon.shutdown")
            process.stdin.close()
            process.wait(timeout=120)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
        self.assert_not_killed(process.returncode, ["daemon"], process.stderr.read())
        self.assertEqual(process.returncode, 0)
        self.assert_no_resolver_calls()


if __name__ == "__main__":
    unittest.main()
