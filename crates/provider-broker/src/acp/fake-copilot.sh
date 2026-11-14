#!/bin/sh
# A fake GitHub Copilot CLI for the ACP adapter's tests. It lives in a
# directory of its own and replays that directory's `transcript`, one step
# per line, recording what Kassiber gave it:
#
#   argv      its arguments, one per line (ACP launches only)
#   env       the names in its environment, sorted
#   home      $COPILOT_HOME;  allow-all  $COPILOT_ALLOW_ALL or "unset"
#   cwd       its working directory
#   inputs    every line read from stdin
#   mcp-out   what the Kassiber MCP server answered (step `mcp`)
#
# Steps:
#   <                  read one stdin line
#   > JSON             write JSON as one stdout line
#   err TEXT           write TEXT to stderr
#   exit N             exit with status N
#   linger N           leave a process holding stdout open for N seconds
#   mcp TOOL ARGS      start the MCP server from --additional-mcp-config and
#                      call TOOL with the JSON object ARGS through it
#   exit-marker PATH   on SIGTERM, write "exited" to PATH 300 ms later, then
#                      exit (a process slow to let go of its directory)
#
# After the last step it reads stdin until it ends (or, with an exit marker,
# waits for SIGTERM). `--version` prints the file `version`.
dir=$(cd "$(dirname "$0")" && pwd)
if [ "$1" = "--version" ]; then
  cat "$dir/version"
  exit 0
fi
: > "$dir/argv"
for arg in "$@"; do
  printf '%s\n' "$arg" >> "$dir/argv"
done
env | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' | sort > "$dir/env"
printf '%s\n' "${COPILOT_HOME-}" > "$dir/home"
printf '%s\n' "${COPILOT_ALLOW_ALL-unset}" > "$dir/allow-all"
pwd > "$dir/cwd"
: > "$dir/inputs"

mcp_call() {
  config=$(sed -n '/^--additional-mcp-config$/{n;p;}' "$dir/argv")
  server=$(printf '%s' "$config" | sed 's/.*"command":"\([^"]*\)".*/\1/')
  server_args=$(printf '%s' "$config" | sed 's/.*"args":\["\(.*\)"\],"tools".*/\1/; s/","/ /g')
  rm -f "$dir/mcp-out" "$dir/mcp-done"
  {
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fake-copilot","version":"1"}}}'
    printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"%s","arguments":%s}}\n' "$1" "$2"
    while [ ! -e "$dir/mcp-done" ]; do sleep 0.05; done
  } | "$server" $server_args > "$dir/mcp-out" &
  pipeline=$!
  tries=0
  until grep -q '"id":2' "$dir/mcp-out" 2>/dev/null || [ "$tries" -ge 400 ]; do
    sleep 0.05
    tries=$((tries + 1))
  done
  : > "$dir/mcp-done"
  wait "$pipeline"
}

marker=
while IFS= read -r step <&3; do
  case "$step" in
    "<") IFS= read -r line || exit 0; printf '%s\n' "$line" >> "$dir/inputs" ;;
    "> "*) printf '%s\n' "${step#> }" ;;
    "err "*) printf '%s\n' "${step#err }" >&2 ;;
    "exit "*) exit "${step#exit }" ;;
    "linger "*) sleep "${step#linger }" & ;;
    "mcp "*)
      rest=${step#mcp }
      mcp_call "${rest%% *}" "${rest#* }"
      ;;
    "exit-marker "*)
      marker=${step#exit-marker }
      trap 'kill "$sleeper" 2>/dev/null; sleep 0.3; echo exited > "$marker"; exit 0' TERM
      ;;
  esac
done 3< "$dir/transcript"

if [ -n "$marker" ]; then
  while :; do
    sleep 1 &
    sleeper=$!
    wait "$sleeper"
  done
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/inputs"
done
