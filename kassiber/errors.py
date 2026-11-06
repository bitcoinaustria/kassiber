"""Structured exception type used throughout kassiber.

`AppError` carries the fields the JSON envelope expects: `code`, `hint`,
`details`, `retryable`. The CLI translates any uncaught `AppError` into a
machine-readable error envelope via `build_error_envelope` in
`kassiber.envelope`.

Call sites raise with enough context that the error envelope is actionable:

    raise AppError(
        "Rate must be positive",
        code="validation",
        hint="Pass a positive number",
    )
"""


class AppError(Exception):
    """Raised for user-facing, CLI-recoverable errors.

    Uncaught AppErrors are caught at the dispatch boundary and rendered as
    a JSON error envelope. Internal programmer errors (bad types, impossible
    state) should use plain assertions / TypeError / ValueError so they show
    as unhandled tracebacks in `--debug` mode.
    """

    def __init__(self, message, code="app_error", details=None, hint=None, retryable=False):
        super().__init__(message)
        self.code = code
        self.details = details
        self.hint = hint
        self.retryable = retryable


# A blocking error names a condition; ``details["remediation"]`` names what
# resolves it, on which record, and with which tool. Callers (the CLI envelope,
# the desktop, and the in-app AI's tool results) can act on it without
# inferring the target from free text. ``tool`` is the in-app AI's function
# name, or ``None`` when no tool can take the step; ``desktop_only`` marks a
# step only the user can take in the desktop.
REMEDIATION_KEY = "remediation"


def build_remediation(
    action,
    *,
    target=None,
    tool=None,
    arguments_hint=None,
    desktop_only=False,
    message=None,
    alternatives=None,
):
    """Return the shared remediation shape for ``AppError.details``.

    ``target`` holds full, resolvable ids only (never display forms), and
    ``arguments_hint`` is a starting point the caller must still review; it
    never replaces the plan and consent steps the named tool requires.
    """

    remediation = {
        "action": str(action),
        "target": dict(target or {}),
        "tool": tool,
        "arguments_hint": dict(arguments_hint or {}),
        "desktop_only": bool(desktop_only),
    }
    if message:
        remediation["message"] = str(message)
    if alternatives:
        remediation["alternatives"] = [dict(item) for item in alternatives]
    return remediation


def error_remediation(error):
    """The remediation an ``AppError`` carries, or ``None``."""

    details = getattr(error, "details", None)
    if isinstance(details, dict):
        value = details.get(REMEDIATION_KEY)
        if isinstance(value, dict):
            return value
    return None
