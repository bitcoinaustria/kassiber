"""Cooperative transport checks without using SQLite from network threads."""

from concurrent.futures import ThreadPoolExecutor
from contextvars import ContextVar, copy_context
from queue import Empty, Queue
from threading import Event


sync_transport_check = ContextVar("sync_transport_check", default=None)


def check_sync_transport():
    check = sync_transport_check.get()
    if check is not None:
        check()


def run_sync_network(step, check=None):
    """Run preflight I/O while its caller services database checks."""
    check = check if check is not None else sync_transport_check.get()
    if check is None:
        return step()
    result = checked_parallel_map(lambda _item: step(), [None], lambda _item: check(), 1)[0]
    if isinstance(result, BaseException):
        raise result
    return result


def checked_parallel_map(step, items, check, max_workers):
    """Service worker checks on the caller that owns the database connection.

    Each transport waits for a fresh check, including queued HTTP work and
    pooled Electrum batches. All workers finish before this function returns.
    """
    requests = Queue()

    def ask_owner(item):
        reply = {"ready": Event(), "error": None, "item": item}
        requests.put(reply)
        reply["ready"].wait()
        if reply["error"] is not None:
            raise reply["error"]

    def run(item):
        token = sync_transport_check.set(lambda: ask_owner(item))
        try:
            check_sync_transport()
            return step(item)
        except Exception as exc:
            return exc
        finally:
            sync_transport_check.reset(token)

    with ThreadPoolExecutor(max_workers=max_workers) as executor:
        futures = [executor.submit(copy_context().run, run, item) for item in items]
        interrupted = None
        while not all(future.done() for future in futures):
            try:
                reply = requests.get(timeout=0.01)
            except Empty:
                continue
            except BaseException as exc:
                interrupted = exc
                continue
            try:
                if interrupted is not None:
                    raise interrupted
                check(reply["item"])
            except BaseException as exc:
                reply["error"] = exc
                if not isinstance(exc, Exception):
                    interrupted = exc
            finally:
                reply["ready"].set()
        if interrupted is not None:
            raise interrupted
        return [future.result() for future in futures]
