# Sourced by the fake provider CLIs: record this invocation under
# "$fixture/calls/<n>-<pid>/" (argv NUL-separated, the environment's names,
# a few values, the working directory) so tests can compare what each broker
# passed. `fixture` is the directory holding the fake.
PATH=/usr/bin:/bin
calls="$fixture/calls"
mkdir -p "$calls"
call="$calls/$(ls "$calls" | wc -l | tr -d ' ')-$$"
mkdir "$call"
printf '%s\0' "$@" > "$call/argv"
awk 'BEGIN { for (name in ENVIRON) print name }' | sort > "$call/env-names"
for name in KASSIBER_AI_BROKER RUST_LOG CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC \
    DISABLE_TELEMETRY DISABLE_ERROR_REPORTING; do
  eval "value=\${$name-<unset>}"
  printf '%s=%s\n' "$name" "$value"
done > "$call/env-values"
pwd > "$call/cwd"
