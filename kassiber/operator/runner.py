"""Short-lived direct CLI runner used by serialized project workers."""

from __future__ import annotations

import os
import subprocess
import threading

from .launcher import cli_child_command, prepare_independent_child_environment
from .project import canonical_project
from .service import Operation, OperationResult


def run_cli_operation(operation: Operation, passphrase: bytearray) -> OperationResult:
    if canonical_project(operation.data_root).identity != operation.project_identity:
        raise RuntimeError("operator project changed before child launch")
    argv, removed_database_secrets = strip_database_passphrase_arguments(
        operation.argv
    )
    read_fds: list[int] = []
    secret_writers: list[tuple[int, bytearray]] = []
    child_handles: list[int] = []
    process: subprocess.Popen[bytes] | None = None
    try:
        db_fd, db_writer, db_token = _secret_pipe()
        read_fds.append(db_fd)
        secret_writers.append((db_writer, passphrase))
        if os.name == "nt":
            child_handles.append(db_token)
            child_handles.extend(operation.owner_handle_tokens)
        argv = ["--db-passphrase-fd", str(db_token), *argv]
        for marker, secret in operation.secret_arguments.items():
            if marker in removed_database_secrets and marker not in argv:
                continue
            secret_fd, secret_writer, secret_token = _secret_pipe()
            read_fds.append(secret_fd)
            secret_writers.append((secret_writer, secret))
            if os.name == "nt":
                child_handles.append(secret_token)
            argv = [str(secret_token) if value == marker else value for value in argv]

        environment = os.environ.copy()
        environment["KASSIBER_OPERATOR_DIRECT"] = "1"
        environment["KASSIBER_OPERATOR_CHILD"] = "1"
        environment["KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"] = (
            operation.project_identity
        )
        environment["KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY"] = (
            operation.database_identity
        )
        prepare_independent_child_environment(environment)
        popen_args: dict[str, object] = {
            "stdin": subprocess.DEVNULL,
            "stdout": subprocess.PIPE,
            "stderr": subprocess.PIPE,
            "env": environment,
        }
        if os.name == "nt":
            startup = subprocess.STARTUPINFO()
            startup.lpAttributeList = {"handle_list": child_handles}
            popen_args["startupinfo"] = startup
            popen_args["close_fds"] = True
        else:
            popen_args["pass_fds"] = tuple(
                [*read_fds, *operation.owner_handle_tokens]
            )
        process = subprocess.Popen(
            [*cli_child_command(), *argv],
            **popen_args,
        )
        operation.process = process
        for fd in read_fds:
            os.close(fd)
        read_fds.clear()
        writers = tuple(secret_writers)
        secret_writers.clear()
        writer_errors: list[Exception] = []
        writer_error_lock = threading.Lock()

        def feed_secret(writer: int, secret: bytearray) -> None:
            try:
                _write_secret(writer, secret)
            except Exception as exc:
                with writer_error_lock:
                    writer_errors.append(exc)
            finally:
                try:
                    os.close(writer)
                except OSError:
                    pass

        writer_threads = [
            threading.Thread(
                target=feed_secret,
                args=(writer, secret),
                name=f"operator-secret-handoff-{index}",
                daemon=True,
            )
            for index, (writer, secret) in enumerate(writers)
        ]
        for writer_thread in writer_threads:
            writer_thread.start()
        # stdout/stderr must be drained while the child reaches its secret-fd
        # reads; otherwise a large secret and a full output pipe can deadlock.
        stdout, stderr = process.communicate()
        for writer_thread in writer_threads:
            writer_thread.join()
        if writer_errors:
            raise writer_errors[0]
        return OperationResult(
            process.returncode,
            stdout.decode("utf-8", errors="replace"),
            stderr.decode("utf-8", errors="replace"),
        )
    except BaseException:
        if process is not None and process.poll() is None:
            process.kill()
            process.communicate()
        raise
    finally:
        operation.process = None
        for fd in read_fds:
            try:
                os.close(fd)
            except OSError:
                pass
        for writer, _secret in secret_writers:
            try:
                os.close(writer)
            except OSError:
                pass


def strip_database_passphrase_arguments(
    argv: list[str],
) -> tuple[list[str], set[str]]:
    """Remove caller DB unlock FDs so the broker lease is authoritative."""

    prepared: list[str] = []
    removed_values: set[str] = set()
    index = 0
    while index < len(argv):
        token = argv[index]
        if token == "--db-passphrase-fd":
            if index + 1 < len(argv):
                removed_values.add(argv[index + 1])
                index += 2
            else:
                index += 1
            continue
        prefix = "--db-passphrase-fd="
        if token.startswith(prefix):
            removed_values.add(token[len(prefix) :])
            index += 1
            continue
        prepared.append(token)
        index += 1
    return prepared, removed_values


def _secret_pipe() -> tuple[int, int, int]:
    read_fd, write_fd = os.pipe()
    if os.name == "nt":
        import msvcrt

        handle = int(msvcrt.get_osfhandle(read_fd))
        os.set_handle_inheritable(handle, True)
        return read_fd, write_fd, handle
    return read_fd, write_fd, read_fd


def _write_secret(fd: int, secret: bytearray) -> None:
    view = memoryview(secret)
    written = 0
    while written < len(view):
        written += os.write(fd, view[written:])
