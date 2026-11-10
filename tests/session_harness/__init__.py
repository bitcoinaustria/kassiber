"""Record CLI and daemon sessions at the process boundary and replay them.

Plan 21's oracle for the Rust backend: `recorder` keeps what a child process
saw and did (argv, environment subset, stdin, fixture files, stdout, stderr,
exit status, and the state roots before and after); `replay` runs the same
session against a candidate executable on a fresh copy of the starting state;
`compare` decides whether the two runs agree. See the "Recorded sessions"
section of docs/reference/testing.md.
"""
