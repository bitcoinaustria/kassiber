"""CLI-only exact-destination export delivery; never a daemon filesystem grant."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
from uuid import uuid4

from ..core.accounting.package import MAX_SNAPSHOT_BYTES, verify_package
from ..errors import AppError

SIDEBAND = 'accounting_local_export'


def _fail():
    raise AppError('Local export delivery failed; the artifact remains prepared. Review and retry.',
                   code='accounting_export_delivery_failed', retryable=True)


def _parent(path):
    """Pin every parent directory without following symlinks; unsupported OS fails closed."""
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in path.parts[1:-1]:
            next_fd = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        return fd
    except BaseException:
        os.close(fd)
        raise


def _save(path, encoded, publication):
    parent = _parent(path)
    temporary = '.kassiber-export-' + uuid4().hex
    created = False
    try:
        try:
            existing = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        except FileNotFoundError:
            existing = None
        if existing is not None:
            with os.fdopen(existing, 'rb') as stream:
                if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode) or stream.read(len(encoded) + 1) != encoded:
                    _fail()
            return
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
        created = True
        with os.fdopen(fd, 'wb') as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        # Hard-link publication is atomic and refuses replacement of any existing target.
        os.link(temporary, path.name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
        publication['may_exist'] = True
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=parent)
        with os.fdopen(fd, 'rb') as stream:
            if stream.read(len(encoded) + 1) != encoded:
                _fail()
        os.fsync(parent)
    finally:
        try:
            if created:
                os.unlink(temporary, dir_fd=parent)
        except FileNotFoundError:
            pass
        os.close(parent)


class Delivery:
    def __init__(self, selections=()):
        self.destinations, self.pending, self.receipts = {}, {}, []
        for task_id, step, filename in selections or ():
            if (not re.fullmatch(r'[a-f0-9]{32}', task_id) or step not in ('export_close', 'export_tax')
                    or not filename or filename == '-' or (task_id, step) in self.destinations):
                _fail()
            path = Path(os.path.abspath(filename))
            if path in self.destinations.values():
                _fail()
            self.destinations[task_id, step] = path

    def destination(self, data):
        review = data.get('accounting_task_preview')
        preview = review.get('preview') if isinstance(review, dict) else None
        if not isinstance(preview, dict):
            return None
        return self.destinations.get((preview.get('id'), preview.get('step')))

    def approve(self, call_id, data):
        path = self.destination(data)
        if path is not None:
            preview = data['accounting_task_preview']['preview']
            self.pending[call_id] = (path, preview['id'], preview['step'], dict(preview['detail']))

    def consume(self, data):
        value = data.get(SIDEBAND)
        approved = self.pending.pop(data.get('call_id'), None)
        if approved is None:
            return  # No exact local approval: discard, never interpret model output.
        path, task_id, step, detail = approved
        publication = {'may_exist': False}
        try:
            if not data.get('ok') or not isinstance(value, dict):
                _fail()
            text = value.get('artifact_json')
            if (value.get('task_id') != task_id or value.get('step') != step or not isinstance(text, str)
                    or len(text) > MAX_SNAPSHOT_BYTES):
                _fail()
            encoded = text.encode('utf-8')
            checksum = hashlib.sha256(encoded).hexdigest()
            if len(encoded) > MAX_SNAPSHOT_BYTES or checksum != value.get('sha256'):
                _fail()
            artifact = json.loads(text)
            identity, digest = ('id', 'snapshot_digest') if step == 'export_close' else ('final_id', 'report_digest')
            if any(artifact.get(key) != detail.get(key) for key in (identity, digest)):
                _fail()
            if step == 'export_close':
                verify_package(artifact)
            elif (artifact.get('stale') is not False
                  or hashlib.sha256(artifact['report_json'].encode('utf-8')).hexdigest() != artifact[digest]
                  or hashlib.sha256(artifact['html'].encode('utf-8')).hexdigest() != artifact['html_sha256']):
                _fail()
            _save(path, encoded, publication)
            receipt = dict(file_saved=True, file_verified=True, path=str(path), sha256=checksum,
                           artifact_id=artifact[identity], content_digest=artifact[digest])
        except (AppError, OSError, ValueError, TypeError, KeyError, AttributeError, UnicodeError):
            receipt = dict(file_saved=None if publication['may_exist'] else False, file_verified=False,
                           may_exist=publication['may_exist'], artifact_prepared=bool(data.get('ok')),
                           code='accounting_export_delivery_failed', retry='Fresh local approval required; never overwrite a conflicting file.')
        self.receipts.append(receipt)

    def render(self, out):
        for _ in self.pending:
            self.receipts.append(dict(file_saved=False, file_verified=False, artifact_prepared=None,
                code='accounting_export_not_delivered', retry='Inspect retained task state and obtain fresh local approval.'))
        for receipt in self.receipts:
            out.write('\nLOCAL EXPORT RECEIPT (not model text): ' + json.dumps(receipt, ensure_ascii=True) + '\n')
        out.flush()
        self.receipts.clear()
        self.pending.clear()
