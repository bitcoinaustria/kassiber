"""Passphrase-gated SQLCipher integration for Kassiber.

The module is split into:

- `prompt`: read passphrases from a controlling TTY or a numbered file
  descriptor; never accept secret values via argv or process environment.
- `sqlcipher`: open the SQLCipher database with the correct PRAGMA order,
  perform the read-`sqlite_master` verification step that proves the key
  was correct, and rekey an already-encrypted handle in place.
- `migration`: take an existing plaintext SQLite store and produce an
  encrypted store with `sqlcipher_export()`, preserving metadata that the
  export does not carry by itself.
- `passphrase`: change-passphrase entry point that wraps `PRAGMA rekey`.
- `credentials`: scan the plaintext `backends.env` bootstrap for
  secret-shaped entries (tokens, passwords, auth headers) and lift them
  into the encrypted `backends` table so nothing secret is left on disk
  in plaintext form.

`secrets/cli.py` exposes the `kassiber secrets {init, change-passphrase,
verify, status}` argparse surface.
"""

from .sqlcipher import (
    KDF_ITER_DEFAULT,
    CIPHER_PAGE_SIZE_DEFAULT,
    CIPHER_COMPATIBILITY,
    apply_keying,
    escape_passphrase,
    open_encrypted,
    rekey_connection,
    verify_unlock,
)
from .prompt import (
    PassphraseInputError,
    prompt_passphrase,
    prompt_passphrase_with_confirmation,
    read_passphrase_from_fd,
    validate_passphrase,
)

# `credentials` is intentionally NOT re-exported here: it depends on
# `kassiber.backends`, which in turn depends on `kassiber.db`, which
# imports this module during its own load. Callers that need the
# credential-migration helpers should `from kassiber.secrets.credentials
# import ...` directly.

__all__ = [
    "KDF_ITER_DEFAULT",
    "CIPHER_PAGE_SIZE_DEFAULT",
    "CIPHER_COMPATIBILITY",
    "PassphraseInputError",
    "apply_keying",
    "escape_passphrase",
    "open_encrypted",
    "prompt_passphrase",
    "prompt_passphrase_with_confirmation",
    "read_passphrase_from_fd",
    "rekey_connection",
    "validate_passphrase",
    "verify_unlock",
]
