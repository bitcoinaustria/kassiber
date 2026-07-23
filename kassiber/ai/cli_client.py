"""Fixed Claude Code and Codex CLI adapters for Kassiber chat."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator

from ..errors import AppError
from .contracts import ChatDelta, DEFAULT_TIMEOUT_SECONDS
from .model_metadata import MODEL_SUPPORT_STRING_LIMIT, safe_string_list


CLI_DEFAULT_MODEL = "default"
REASONING_EFFORTS = {"low", "medium", "high", "max"}
CLI_MODEL_CHECK_KIND = "binary_presence"
CLI_FALLBACK_DIRS = (
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "~/.local/bin",
    "/opt/local/bin",
)
CLI_MODEL_LIST_TIMEOUT_SECONDS = 10
_CLI_BASE_ENV_NAMES = {
    "HOME",
    "LANG",
    "LC_ALL",
    "PATH",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "TERM",
    "TMPDIR",
    "USER",
    "USERNAME",
}
_CLI_PROXY_ENV_NAMES = {
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "all_proxy",
    "https_proxy",
    "http_proxy",
    "no_proxy",
}
_CLAUDE_ENV_NAMES = {
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "ANTHROPIC_VERTEX_REGION",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLOUD_ML_REGION",
    "CLOUDSDK_CORE_PROJECT",
    "GCLOUD_PROJECT",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_QUOTA_PROJECT",
}
_CLAUDE_ENV_PREFIXES = ("AWS_",)
_CODEX_ENV_NAMES = {
    "CODEX_ACCESS_TOKEN",
    "CODEX_API_KEY",
    "CODEX_HOME",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
}
CLAUDE_CLI_MODEL_ROWS = (
    {"id": CLI_DEFAULT_MODEL, "check_kind": CLI_MODEL_CHECK_KIND},
    {"id": "sonnet", "check_kind": "claude_cli_alias"},
    {"id": "opus", "check_kind": "claude_cli_alias"},
)


def _messages_to_prompt(messages: list[dict]) -> str:
    lines: list[str] = []
    for message in messages:
        role = str(message.get("role") or "user").strip() or "user"
        content = message.get("content")
        if not isinstance(content, str) or not content:
            continue
        lines.append(f"{role.upper()}:\n{content}")
    return "\n\n".join(lines).strip()


def _cli_unavailable(command: str) -> AppError:
    return AppError(
        f"AI CLI provider '{command}' is not installed or not on PATH",
        code="ai_unavailable",
        hint=f"Install and authenticate `{command}` before using this provider.",
        retryable=True,
    )


def _resolve_cli_executable(command: str) -> str | None:
    resolved = shutil.which(command)
    if resolved:
        return resolved

    for directory in CLI_FALLBACK_DIRS:
        candidate = Path(directory).expanduser() / command
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def _cli_default_model_row() -> dict[str, Any]:
    return {
        "id": CLI_DEFAULT_MODEL,
        "check_kind": CLI_MODEL_CHECK_KIND,
        "supports_reasoning_effort": True,
        "reasoning_efforts": ["low", "medium", "high"],
    }


def _codex_catalog_models(executable: str) -> list[dict[str, Any]]:
    try:
        completed = subprocess.run(
            [executable, "debug", "models"],
            text=True,
            capture_output=True,
            timeout=CLI_MODEL_LIST_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return []
    if completed.returncode != 0:
        return []

    try:
        payload = json.loads(completed.stdout or "{}")
    except json.JSONDecodeError:
        return []

    raw_models = payload.get("models")
    if not isinstance(raw_models, list):
        return []

    rows: list[dict[str, Any]] = []
    for item in raw_models:
        if not isinstance(item, dict) or item.get("visibility") != "list":
            continue
        slug = item.get("slug")
        if not isinstance(slug, str) or not slug.strip():
            continue
        raw_levels = item.get("supported_reasoning_levels")
        if not isinstance(raw_levels, list):
            raw_levels = []
        supported_efforts = safe_string_list(
            [level.get("effort") for level in raw_levels if isinstance(level, dict)]
        )
        row: dict[str, Any] = {
            "id": slug.strip(),
            "check_kind": "codex_model_catalog",
        }
        display_name = item.get("display_name")
        if isinstance(display_name, str) and display_name.strip():
            row["display_name"] = display_name.strip()[:MODEL_SUPPORT_STRING_LIMIT]
        if supported_efforts:
            row["supports_reasoning_effort"] = True
            row["reasoning_efforts"] = supported_efforts
        rows.append(row)
    return rows


def _cli_failure(command: str, completed: subprocess.CompletedProcess[str]) -> AppError:
    stderr = completed.stderr or ""
    stdout = completed.stdout or ""
    details: dict[str, Any] = {"exit_code": completed.returncode}
    if stderr:
        details["stderr_bytes"] = len(stderr.encode("utf-8", errors="replace"))
    if stdout:
        details["stdout_bytes"] = len(stdout.encode("utf-8", errors="replace"))
    return AppError(
        f"AI CLI provider '{command}' failed",
        code="ai_request_invalid",
        hint=(
            f"Run `{command} --help` or check that the CLI is authenticated. "
            "Prompts sent through Claude/Codex CLI may leave this device."
        ),
        details=details,
        retryable=False,
    )


def _reasoning_effort(options: dict[str, Any] | None) -> str | None:
    if not isinstance(options, dict):
        return None
    raw = options.get("reasoning_effort")
    if not isinstance(raw, str):
        return None
    effort = raw.strip().lower()
    return effort if effort in REASONING_EFFORTS else None


def _cli_subprocess_env(command: str) -> dict[str, str]:
    """Return a minimal environment for external AI CLI subprocesses."""

    allowed = set(_CLI_BASE_ENV_NAMES)
    allowed.update(_CLI_PROXY_ENV_NAMES)
    allowed_prefixes: tuple[str, ...] = ()
    if command == "claude":
        allowed.update(_CLAUDE_ENV_NAMES)
        allowed_prefixes = _CLAUDE_ENV_PREFIXES
    elif command == "codex":
        allowed.update(_CODEX_ENV_NAMES)
    env = {
        key: value
        for key, value in os.environ.items()
        if key in allowed or any(key.startswith(prefix) for prefix in allowed_prefixes)
    }
    env.setdefault("NO_COLOR", "1")
    return env


@dataclass
class CliAIClient:
    """Narrow, non-persistent adapter for Claude Code and Codex CLI."""

    locator: str
    timeout: float = DEFAULT_TIMEOUT_SECONDS

    @property
    def command(self) -> str:
        if self.locator == "claude-cli://default":
            return "claude"
        if self.locator == "codex-cli://default":
            return "codex"
        raise AppError(
            f"Unsupported AI CLI provider locator '{self.locator}'",
            code="validation",
            hint="Use claude-cli://default or codex-cli://default.",
        )

    def list_models(self, *, strict: bool = False) -> list[dict]:
        del strict
        executable = _resolve_cli_executable(self.command)
        if not executable:
            raise _cli_unavailable(self.command)
        if self.command == "codex":
            return _codex_catalog_models(executable) or [_cli_default_model_row()]
        return [dict(row) for row in CLAUDE_CLI_MODEL_ROWS]

    def _claude_args(
        self,
        *,
        command: str | None = None,
        model: str,
        effort: str | None,
    ) -> list[str]:
        args = [
            command or self.command,
            "--print",
            "--no-session-persistence",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--output-format",
            "json",
        ]
        if model and model != CLI_DEFAULT_MODEL:
            args.extend(["--model", model])
        if effort:
            args.extend(["--effort", effort])
        return args

    def _codex_args(
        self,
        *,
        command: str | None = None,
        cwd: str,
        output_path: str,
        model: str,
        effort: str | None,
    ) -> list[str]:
        args = [
            command or self.command,
            "exec",
            "--sandbox",
            "read-only",
            "--cd",
            cwd,
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-rules",
            "--color",
            "never",
            "--output-last-message",
            output_path,
        ]
        if model and model != CLI_DEFAULT_MODEL:
            args.extend(["--model", model])
        if effort:
            args.extend(["-c", f'model_reasoning_effort="{effort}"'])
        args.append("-")
        return args

    def _run(
        self,
        *,
        prompt: str,
        model: str,
        options: dict[str, Any] | None = None,
    ) -> str:
        command = self.command
        executable = _resolve_cli_executable(command)
        if not executable:
            raise _cli_unavailable(command)
        env = _cli_subprocess_env(command)
        effort = _reasoning_effort(options)
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-cli-") as cwd:
            if command == "claude":
                args = self._claude_args(command=executable, model=model, effort=effort)
                completed = subprocess.run(
                    args,
                    input=prompt,
                    text=True,
                    capture_output=True,
                    cwd=cwd,
                    env=env,
                    timeout=self.timeout,
                    check=False,
                )
                if completed.returncode != 0:
                    raise _cli_failure(command, completed)
                try:
                    payload = json.loads(completed.stdout or "{}")
                except json.JSONDecodeError:
                    return (completed.stdout or "").strip()
                result = payload.get("result") if isinstance(payload, dict) else None
                if isinstance(result, str):
                    return result.strip()
                return (completed.stdout or "").strip()

            with tempfile.NamedTemporaryFile("r", encoding="utf-8", delete=False) as output:
                output_path = output.name
            try:
                args = self._codex_args(
                    command=executable,
                    cwd=cwd,
                    output_path=output_path,
                    model=model,
                    effort=effort,
                )
                completed = subprocess.run(
                    args,
                    input=prompt,
                    text=True,
                    capture_output=True,
                    cwd=cwd,
                    env=env,
                    timeout=self.timeout,
                    check=False,
                )
                if completed.returncode != 0:
                    raise _cli_failure(command, completed)
                try:
                    with open(output_path, "r", encoding="utf-8") as handle:
                        content = handle.read().strip()
                except OSError:
                    content = ""
                return content or (completed.stdout or "").strip()
            finally:
                try:
                    os.unlink(output_path)
                except OSError:
                    pass

    def chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        if tools or tool_choice not in (None, "none"):
            raise AppError(
                "CLI AI providers cannot be used with Kassiber tools enabled",
                code="ai_cli_tools_disabled",
                hint=(
                    "Turn off assistant tools for Claude/Codex CLI providers, "
                    "or use an OpenAI Responses-compatible provider so Kassiber can enforce "
                    "the typed tool allowlist and consent gates."
                ),
                retryable=False,
            )
        content = self._run(
            prompt=_messages_to_prompt(messages),
            model=model,
            options=options,
        )
        return {
            "role": "assistant",
            "content": content,
            "finish_reason": "stop",
            "usage": None,
        }

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> Iterator[ChatDelta]:
        response = self.chat(
            messages=messages,
            model=model,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
        )
        yield ChatDelta(
            delta={"role": "assistant", "content": response["content"]},
            finish_reason=response.get("finish_reason"),
            raw={"provider": self.command},
        )
