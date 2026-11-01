"""Cooperative transport checks without using SQLite from network threads."""

from concurrent.futures import ThreadPoolExecutor
from contextvars import ContextVar, copy_context
from queue import Empty, Queue
from threading import Event


sync_transport_check = ContextVar("sync_transport_check", default=None)
sync_progress_emitter = ContextVar("kassiber.sync_progress_emitter", default=None)


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
    """Service worker checks and progress on the database-owning caller.

    Each transport waits for a fresh check, including queued HTTP work and
    pooled Electrum batches. All workers finish before this function returns.
    """
    requests = Queue()
    progress = sync_progress_emitter.get()

    def ask_owner(item, *, payload=None, is_progress=False):
        reply = {"ready": Event(), "error": None, "item": item,
                 "payload": payload, "is_progress": is_progress}
        requests.put(reply)
        reply["ready"].wait()
        if reply["error"] is not None:
            raise reply["error"]

    def run(item):
        token = sync_transport_check.set(lambda: ask_owner(item))
        progress_token = sync_progress_emitter.set(
            (lambda payload: ask_owner(item, payload=payload, is_progress=True))
            if progress is not None else None
        )
        try:
            check_sync_transport()
            return step(item)
        except Exception as exc:
            return exc
        finally:
            sync_progress_emitter.reset(progress_token)
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
                if reply["is_progress"]:
                    progress(reply["payload"])
                else:
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
