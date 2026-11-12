//! Handed-over descriptors reach the child with their own contents even when
//! a source pipe holds the number of another target (the replay's descriptor
//! numbers are whatever its other threads left free).
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::process::Command;

use kassiber_session_replay::{close_sources, filled_pipe, hand_over};

/// Move `fd` to exactly `number`, which must be free.
fn place(fd: i32, number: i32) -> i32 {
    // SAFETY: probing and duplicating descriptors this test owns.
    unsafe {
        assert_eq!(libc::fcntl(number, libc::F_GETFD), -1, "fd {number} in use");
        assert_eq!(libc::dup2(fd, number), number);
        libc::close(fd);
    }
    number
}

#[test]
fn swapped_sources_and_targets_keep_their_contents() {
    // Target 200 gets "first" from a pipe now at 201, and target 201 gets
    // "second" from a pipe now at 200: an in-order dup2 would copy 201 onto
    // 200 and lose "second".
    let first = place(filled_pipe(b"first").unwrap(), 201);
    let second = place(filled_pipe(b"second").unwrap(), 200);
    let handed = vec![(200, first), (201, second)];
    let mut command = Command::new("/bin/bash");
    command.args(["-c", "cat <&200; printf '|'; cat <&201"]);
    hand_over(&mut command, handed.clone());
    let output = command.output().unwrap();
    close_sources(&handed);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "first|second");
}

#[test]
fn filled_pipes_do_not_leak_into_other_children() {
    let pipe = filled_pipe(b"private").unwrap();
    let output = Command::new("/bin/bash")
        .args([
            "-c",
            &format!("cat <&{pipe} 2>/dev/null && echo leaked || echo closed"),
        ])
        .output()
        .unwrap();
    close_sources(&[(0, pipe)]);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "closed");
}
