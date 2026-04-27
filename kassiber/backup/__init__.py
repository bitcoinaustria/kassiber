"""Single-file Kassiber backup format: `tar | age`.

The export path streams a tar archive through an `age` subprocess (or
`pyrage` when no system binary is available) into a `.kassiber` file.
The archive contains an in-place SQLCipher copy of the database, the
managed attachments tree, the backends config file, and a manifest.

The import path is the inverse, plus a strict `tarfile`-member validator
because the Python stdlib documentation explicitly warns that
`tarfile.extractall()` is unsafe on untrusted archives even with the
default `data` filter.
"""

from .age_cli import (
    AgeBackend,
    AgeUnavailableError,
    decrypt_age_stream,
    encrypt_age_stream,
    select_age_backend,
)
from .pack import (
    BACKUP_DB_NAME,
    BACKUP_MANIFEST_NAME,
    MANIFEST_SCHEMA_VERSION,
    BackupExportResult,
    BackupImportResult,
    export_backup,
    import_backup,
)
from .safe_tar import (
    UnsafeTarMember,
    extract_tar_safely,
    inspect_tar_members,
)

__all__ = [
    "AgeBackend",
    "AgeUnavailableError",
    "BACKUP_DB_NAME",
    "BACKUP_MANIFEST_NAME",
    "BackupExportResult",
    "BackupImportResult",
    "MANIFEST_SCHEMA_VERSION",
    "UnsafeTarMember",
    "decrypt_age_stream",
    "encrypt_age_stream",
    "export_backup",
    "extract_tar_safely",
    "import_backup",
    "inspect_tar_members",
    "select_age_backend",
]
