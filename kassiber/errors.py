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
