//! Same-user local IPC and child-process primitives for the operator broker
//! (docs/reference/r6-ai-operator-port.md, R6-1).
//!
//! This crate holds every platform call the broker, its clients and its
//! children need, and nothing else:
//!
//! - [`unix`]: `AF_UNIX` stream sockets bound with an explicit mode and
//!   backlog, a connect bounded by a timeout, and the peer's credentials
//!   (`SO_PEERCRED` on Linux, `getpeereid` plus `LOCAL_PEERPID` on macOS);
//! - [`windows`]: named pipes with an owner-only DACL,
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`,
//!   overlapped I/O with deadlines and SID checks on both ends;
//! - [`process`]: child launches that inherit exactly the listed descriptors
//!   or handles, optionally detached into a new session;
//! - [`signal`]: termination signals delivered to a flag and a wake-up
//!   descriptor;
//! - [`poll`]: waiting for a pipe to become readable with a timeout.
//!
//! The only sockets it creates are `AF_UNIX`; it never resolves names or
//! opens a network route; the workspace chokepoint scanner holds it to that.
//! Policy (which uid or SID is acceptable, what an error is called) belongs
//! to `kassiber-operator`; this crate reports what the OS says.

pub mod poll;
pub mod process;
pub mod signal;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;
