#!/usr/bin/env bash
# A packaged Kassiber binary opens no network socket on launch, status, or idle.
#
# The production binary has no test egress observer (an armed
# KASSIBER_TEST_NO_EGRESS makes it exit 78), so this check watches it from the
# operating system instead. Every child runs under a seccomp filter that kills
# the process on any socket(AF_INET | AF_INET6 | AF_PACKET) call, loopback
# included, and on io_uring_setup; an LD_PRELOAD shim records every libc
# resolver call (getaddrinfo, gethostbyname*, getnameinfo), because glibc can
# resolve over systemd-resolved's Unix socket without opening an IP socket.
#
# Product egress stays enabled and the book carries saved backends (an
# unresolvable .invalid Esplora and a loopback Electrum), so nothing but the
# product's own consent rules keeps it quiet: a saved backend is
# configuration, not consent. The scenarios are launch (--version, --help),
# status on a fresh, a plaintext and a SQLCipher book, and a daemon that
# answers display reads and then sits idle. Two controls first prove the
# watchers fire: deliberate socket and lookup probes, and the binary's own
# user-triggered sync against a loopback Esplora.
#
# Linux x86-64 and AArch64 only; needs a C compiler and jq. Other platforms
# print "skipped:" and exit 0.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: scripts/check-packaged-cli-no-egress.sh --cli PATH --scratch DIR

  --cli PATH      The packaged kassiber executable to check.
  --scratch DIR   An empty or missing directory for disposable books and the
                  compiled watchers.

KASSIBER_PACKAGED_IDLE_SECONDS sets the daemon's idle wait (default 8).
EOF
}

cli=""
scratch=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --cli) cli="${2:-}"; shift 2 ;;
    --scratch) scratch="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done
[ -n "$cli" ] && [ -n "$scratch" ] || { usage; exit 2; }

if [ "$(uname -s)" != Linux ]; then
  echo "skipped: the seccomp watcher supports Linux x86-64 and AArch64 only"
  exit 0
fi
case "$(uname -m)" in
  x86_64|aarch64) ;;
  *) echo "skipped: the seccomp watcher supports Linux x86-64 and AArch64 only"; exit 0 ;;
esac
[ -x "$cli" ] || { echo "Not an executable: $cli" >&2; exit 2; }
compiler="$(command -v cc || command -v gcc || true)"
[ -n "$compiler" ] || { echo "a C compiler is needed for the watchers" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "jq is required for JSON assertions" >&2; exit 2; }
if [ -e "$scratch" ] && [ -n "$(ls -A "$scratch")" ]; then
  echo "Scratch directory is not empty: $scratch" >&2
  exit 2
fi
mkdir -p "$scratch"
scratch="$(cd "$scratch" && pwd -P)"
cli="$(cd "$(dirname "$cli")" && pwd -P)/$(basename "$cli")"
idle_seconds="${KASSIBER_PACKAGED_IDLE_SECONDS:-8}"
passphrase="packaged no-egress passphrase"
descriptor='wpkh([91790c21/84h/0h/0h]xpub6CpT4xRJntYQuFwkiZfSiK7jEtPM3e9nUfJKQ8ATWkgYJui4WS5HmzjD6C7avtwAFtmEyAusdFFxg2rZMs73SoSVkPQfEaQtrLGox92JNWn/0/*)'
# Exit statuses: the shell reports a SIGSYS kill as 128 + 31; the watcher
# exits 121/122 when it cannot install the filter.
sigsys_status=$((128 + 31))

# -- the watchers ------------------------------------------------------------

bin="$scratch/bin"
mkdir -p "$bin"

cat > "$bin/resolver_shim.c" <<'C'
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
C

# watch PROGRAM ARGS...: no new privileges, the seccomp filter, then exec.
cat > "$bin/watch.c" <<'C'
#define _GNU_SOURCE
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef __NR_io_uring_setup
#define __NR_io_uring_setup 425
#endif
#if defined(__x86_64__)
#define WATCH_ARCH AUDIT_ARCH_X86_64
#elif defined(__aarch64__)
#define WATCH_ARCH AUDIT_ARCH_AARCH64
#else
#error "the seccomp watcher supports x86-64 and AArch64 only"
#endif

#define NR_OFFSET offsetof(struct seccomp_data, nr)
#define ARCH_OFFSET offsetof(struct seccomp_data, arch)
/* args[0], the low word on little endian: socket's domain. */
#define ARG0_OFFSET offsetof(struct seccomp_data, args[0])

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: watch PROGRAM [ARGS...]\n");
        return 2;
    }
    struct sock_filter filter[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, ARCH_OFFSET),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, WATCH_ARCH, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),  /* foreign ABI */
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, NR_OFFSET),
#if defined(__x86_64__)
        BPF_JUMP(BPF_JMP | BPF_JGE | BPF_K, 0x40000000, 0, 1),  /* x32 numbers */
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
#endif
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_io_uring_setup, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_socket, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, ARG0_OFFSET),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AF_INET, 3, 0),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AF_INET6, 2, 0),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AF_PACKET, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
    };
    struct sock_fprog program = {
        .len = (unsigned short)(sizeof filter / sizeof filter[0]),
        .filter = filter,
    };
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0) {
        perror("prctl(PR_SET_NO_NEW_PRIVS)");
        return 121;
    }
    if (syscall(__NR_seccomp, SECCOMP_SET_MODE_FILTER, 0, &program) != 0) {
        perror("seccomp(SECCOMP_SET_MODE_FILTER)");
        return 122;
    }
    execvp(argv[1], argv + 1);
    perror("execvp");
    return 127;
}
C

# probe inet|inet6|unix|lookup HOST: the controls' deliberate attempts.
cat > "$bin/probe.c" <<'C'
#include <netdb.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc >= 2 && strcmp(argv[1], "inet") == 0) {
        return socket(AF_INET, SOCK_STREAM, 0) < 0 ? 1 : 0;
    }
    if (argc >= 2 && strcmp(argv[1], "inet6") == 0) {
        return socket(AF_INET6, SOCK_DGRAM, 0) < 0 ? 1 : 0;
    }
    if (argc >= 2 && strcmp(argv[1], "unix") == 0) {
        return socket(AF_UNIX, SOCK_STREAM, 0) < 0 ? 1 : 0;
    }
    if (argc >= 3 && strcmp(argv[1], "lookup") == 0) {
        struct addrinfo *result = NULL;
        if (getaddrinfo(argv[2], "443", NULL, &result) == 0 && result) freeaddrinfo(result);
        return 0;
    }
    fprintf(stderr, "usage: probe inet|inet6|unix|lookup HOST\n");
    return 2;
}
C

"$compiler" -shared -fPIC -O2 -o "$bin/resolver_shim.so" "$bin/resolver_shim.c"
"$compiler" -O2 -o "$bin/watch" "$bin/watch.c"
"$compiler" -O2 -o "$bin/probe" "$bin/probe.c"

# -- per-case environment ----------------------------------------------------

# The production binary refuses an armed test guard (exit 78); the product
# switch would hide exactly what this check watches. Saved-backend overrides
# from the caller's environment must not reach the book either.
scrub=(-u KASSIBER_TEST_NO_EGRESS -u KASSIBER_NO_EGRESS -u LD_PRELOAD)
while IFS= read -r name; do
  case "$name" in
    KASSIBER_BACKEND_*|SATBOOKS_BACKEND_*) scrub+=(-u "$name") ;;
  esac
done < <(compgen -e)

case_dir=""
data_root=""
resolver_log=""
failures=0

new_case() {
  case_dir="$scratch/cases/$1"
  mkdir -p "$case_dir/preferences"
  data_root="$case_dir/data"
  resolver_log="$case_dir/resolver.log"
  echo "no-egress: $1"
}

# in_case COMMAND...: the case's environment, without the watcher.
in_case() {
  (
    cd "$case_dir"
    exec env "${scrub[@]}" \
      XDG_DATA_HOME="$case_dir/xdg-data" \
      KASSIBER_OFFLINE_PREFERENCE_FILE="$case_dir/preferences/offline-mode.json" \
      KASSIBER_AGENT_ACCESS_FILE="$case_dir/preferences/agent-access.json" \
      KASSIBER_UPDATE_PREFERENCE_FILE="$case_dir/preferences/update-check.json" \
      KASSIBER_UPDATE_CACHE_FILE="$case_dir/preferences/update-cache.json" \
      LD_PRELOAD="$bin/resolver_shim.so" \
      KASSIBER_RESOLVER_SHIM_LOG="$resolver_log" \
      TMPDIR="$case_dir" \
      "$@"
  )
}

# watched COMMAND...: the case's environment under the seccomp watcher.
watched() {
  in_case "$bin/watch" "$@"
}

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

# check_status STATUS WHAT STDERR_FILE: the watcher must not have fired.
check_status() {
  local status="$1" what="$2" stderr_file="${3:-}"
  if [ "$status" -eq 121 ] || [ "$status" -eq 122 ]; then
    fail "the seccomp filter could not be installed ($what)"
    return 1
  fi
  if [ "$status" -eq "$sigsys_status" ]; then
    fail "$what opened an IP socket (killed by the seccomp watcher)"
    [ -n "$stderr_file" ] && [ -f "$stderr_file" ] && tail -c 2000 "$stderr_file" >&2
    return 1
  fi
  return 0
}

# kassiber ARGS...: one watched `--machine` run on the case's book; stdout to
# $case_dir/out.json. Fails the check on a nonzero exit unless KASSIBER_MAY_FAIL=1.
kassiber() {
  local status=0
  watched "$cli" --data-root "$data_root" --machine "$@" \
    > "$case_dir/out.json" 2> "$case_dir/err.txt" || status=$?
  check_status "$status" "$*" "$case_dir/err.txt" || return 1
  if [ "$status" -ne 0 ] && [ "${KASSIBER_MAY_FAIL:-0}" != 1 ]; then
    fail "$* exited $status: $(cat "$case_dir/out.json" "$case_dir/err.txt")"
    return 1
  fi
  return 0
}

# json_is FILTER EXPECTED: jq FILTER over the last output equals EXPECTED.
json_is() {
  local actual
  actual="$(jq -r "$1" "$case_dir/out.json" 2>/dev/null || true)"
  if [ "$actual" != "$2" ]; then
    fail "$1: expected '$2', got '$actual' ($(head -c 2000 "$case_dir/out.json"))"
    return 1
  fi
}

assert_no_resolver_calls() {
  if [ -s "$resolver_log" ]; then
    fail "resolver calls recorded in $1:"
    cat "$resolver_log" >&2
  fi
}

seed_book_with_backends() {
  kassiber init &&
  kassiber workspaces create Smoke &&
  kassiber profiles create --workspace Smoke --fiat-currency EUR --tax-country generic Main &&
  kassiber backends create --kind esplora --url https://egress-probe.invalid/api dnsesplora &&
  kassiber backends create --kind electrum --url tcp://127.0.0.1:9 loopelectrum &&
  kassiber backends set-default dnsesplora &&
  kassiber wallets create --workspace Smoke --profile Main --label Watched --kind address \
    --backend dnsesplora --address bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080
}

# -- controls: both watchers fire on a deliberate attempt ---------------------

new_case watchers-catch-a-socket-and-a-lookup
status=0; watched "$bin/probe" inet 2>/dev/null || status=$?
[ "$status" -eq "$sigsys_status" ] || fail "an AF_INET socket was not killed (exit $status)"
status=0; watched "$bin/probe" inet6 2>/dev/null || status=$?
[ "$status" -eq "$sigsys_status" ] || fail "an AF_INET6 datagram socket was not killed (exit $status)"
status=0; watched "$bin/probe" unix || status=$?
[ "$status" -eq 0 ] || fail "an AF_UNIX socket was not allowed (exit $status)"
status=0; in_case "$bin/probe" lookup egress-probe.invalid || status=$?
[ "$status" -eq 0 ] || fail "the lookup probe exited $status"
grep -q "getaddrinfo egress-probe.invalid" "$resolver_log" 2>/dev/null \
  || fail "the resolver shim did not record a deliberate lookup"

new_case watcher-stops-a-user-triggered-sync
if kassiber init &&
   kassiber workspaces create Smoke &&
   kassiber profiles create --workspace Smoke --fiat-currency EUR --tax-country generic Main &&
   kassiber backends create --kind esplora --url http://127.0.0.1:9/api loopesplora &&
   kassiber wallets create --workspace Smoke --profile Main --label Loopback --kind descriptor \
     --backend loopesplora --descriptor-stdin --gap-limit 1 <<<"$descriptor"; then
  status=0
  watched "$cli" --data-root "$data_root" --machine wallets sync \
    --workspace Smoke --profile Main --wallet Loopback \
    > "$case_dir/out.json" 2> "$case_dir/err.txt" || status=$?
  [ "$status" -eq "$sigsys_status" ] \
    || fail "the binary's own sync was not killed by the watcher (exit $status): $(cat "$case_dir/out.json" "$case_dir/err.txt")"
fi

# -- the packaged binary -------------------------------------------------------

new_case launch-and-fresh-status
for flag in --version --help; do
  status=0
  watched "$cli" "$flag" > "$case_dir/out$flag.txt" 2> "$case_dir/err.txt" || status=$?
  check_status "$status" "$flag" "$case_dir/err.txt" || true
done
version_text="$(tr -d '\r' < "$case_dir/out--version.txt" | head -n 1)"
if KASSIBER_MAY_FAIL=1 kassiber status; then
  json_is '.error.code' not_initialized || true
fi
assert_no_resolver_calls "launch and fresh status"

new_case status-with-saved-backends
if seed_book_with_backends && kassiber status; then
  json_is '.kind' status || true
  kassiber backends list || true
  kassiber wallets list --workspace Smoke --profile Main || true
fi
assert_no_resolver_calls "status with saved backends"

new_case encrypted-book-status
if seed_book_with_backends &&
   kassiber secrets init --new-passphrase-fd 0 <<<"$passphrase"; then
  if cmp -s -n 16 "$data_root/kassiber.sqlite3" <(printf 'SQLite format 3\0'); then
    fail "secrets init left a plaintext book"
  fi
  if kassiber --db-passphrase-fd 0 status <<<"$passphrase"; then
    json_is '.kind' status || true
  fi
fi
assert_no_resolver_calls "encrypted book status"

new_case daemon-display-reads-and-idle
if seed_book_with_backends; then
  coproc DAEMON {
    cd "$case_dir"
    exec env "${scrub[@]}" \
      XDG_DATA_HOME="$case_dir/xdg-data" \
      KASSIBER_OFFLINE_PREFERENCE_FILE="$case_dir/preferences/offline-mode.json" \
      KASSIBER_AGENT_ACCESS_FILE="$case_dir/preferences/agent-access.json" \
      KASSIBER_UPDATE_PREFERENCE_FILE="$case_dir/preferences/update-check.json" \
      KASSIBER_UPDATE_CACHE_FILE="$case_dir/preferences/update-cache.json" \
      LD_PRELOAD="$bin/resolver_shim.so" \
      KASSIBER_RESOLVER_SHIM_LOG="$resolver_log" \
      TMPDIR="$case_dir" \
      "$bin/watch" "$cli" --data-root "$data_root" daemon 2> "$case_dir/daemon-err.txt"
  }
  daemon_pid=$DAEMON_PID
  daemon_out="${DAEMON[0]}"
  daemon_in="${DAEMON[1]}"
  daemon_ok=1
  # next_message: one line within 120 s into $case_dir/out.json.
  next_message() {
    local line=""
    if ! IFS= read -r -t 120 line <&"$daemon_out"; then
      daemon_ok=0
      fail "the daemon stopped answering"
      return 1
    fi
    printf '%s\n' "$line" > "$case_dir/out.json"
  }
  send() {
    printf '%s\n' "$1" >&"$daemon_in"
  }
  if next_message && json_is '.kind' daemon.ready; then
    supported="$(jq -r '(.supported_kinds // .data.supported_kinds // [])[]' "$case_dir/out.json")"
    reads=()
    for kind in status ui.overview.snapshot ui.workspace.overview.snapshot ui.wallets.list \
        ui.backends.list ui.backends.settings.list ui.maintenance.settings ui.freshness.status \
        ui.agent_access.status; do
      if grep -qxF "$kind" <<<"$supported"; then
        reads+=("$kind")
      fi
    done
    [ "${reads[0]:-}" = status ] || fail "the daemon does not support status reads"
    index=0
    for kind in "${reads[@]}"; do
      send "{\"request_id\":\"read-$index\",\"kind\":\"$kind\",\"args\":{}}"
      next_message || break
      json_is '.request_id' "read-$index" || true
      index=$((index + 1))
    done
    if [ "$daemon_ok" -eq 1 ]; then
      # Idle: no timer, freshness policy or update check may reach out.
      sleep "$idle_seconds"
      if ! kill -0 "$daemon_pid" 2>/dev/null; then
        fail "the daemon exited while idle"
      else
        send '{"request_id":"after-idle","kind":"status","args":{}}'
        if next_message; then
          json_is '.request_id' after-idle || true
        fi
        send '{"request_id":"shutdown","kind":"daemon.shutdown"}'
        if next_message; then
          json_is '.kind' daemon.shutdown || true
        fi
      fi
    fi
  fi
  exec {daemon_in}>&-
  status=0
  # Bounded wait: a daemon that ignores shutdown is killed and reported.
  for _ in $(seq 1 120); do
    kill -0 "$daemon_pid" 2>/dev/null || break
    sleep 1
  done
  if kill -0 "$daemon_pid" 2>/dev/null; then
    kill "$daemon_pid" 2>/dev/null || true
    fail "the daemon did not exit after shutdown"
  fi
  wait "$daemon_pid" || status=$?
  if check_status "$status" daemon "$case_dir/daemon-err.txt"; then
    [ "$status" -eq 0 ] || fail "the daemon exited $status: $(tail -c 2000 "$case_dir/daemon-err.txt")"
  fi
fi
assert_no_resolver_calls "daemon display reads and idle"

if [ "$failures" -ne 0 ]; then
  echo "Packaged CLI no-egress check failed: $failures problem(s)" >&2
  exit 1
fi
echo "Packaged CLI no-egress check passed: $version_text"
