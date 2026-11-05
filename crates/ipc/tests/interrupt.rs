//! Ctrl-C delivered to the interrupt flag. A test binary of its own, so no
//! other test's handler for SIGINT can race this one.
#![cfg(unix)]

use kassiber_ipc::signal::install_interrupt;
use std::time::{Duration, Instant};

#[test]
fn sigint_sets_the_interrupt_and_cuts_a_sleep_short() {
    let interrupt = install_interrupt().unwrap();
    assert!(!interrupt.is_set());
    let started = Instant::now();
    assert!(!interrupt.sleep(Duration::from_millis(20)));
    assert!(started.elapsed() >= Duration::from_millis(20));
    // SIGINT is caught; the default action would have ended this process.
    raise_sigint();
    assert!(interrupt.is_set());
    let started = Instant::now();
    assert!(interrupt.sleep(Duration::from_secs(5)));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(install_interrupt().unwrap().is_set());

    // Each Ctrl-C is taken once; the next wait sees only a new one.
    assert!(interrupt.take());
    assert!(!interrupt.is_set());
    assert!(!interrupt.take());
    let started = Instant::now();
    assert!(!interrupt.sleep(Duration::from_millis(20)));
    assert!(started.elapsed() >= Duration::from_millis(20));

    // A wait on a quiet descriptor ends at Ctrl-C, and a readable one wins
    // while no Ctrl-C is pending.
    let (reader, mut writer) = std::io::pipe().unwrap();
    use std::os::fd::AsFd;
    let raiser = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(50));
        // SAFETY: kill(2) to this process; the handler is installed.
        assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGINT) }, 0);
    });
    assert!(!interrupt.wait_readable(reader.as_fd()).unwrap());
    raiser.join().unwrap();
    assert!(interrupt.take());
    std::io::Write::write_all(&mut writer, b"x").unwrap();
    assert!(interrupt.wait_readable(reader.as_fd()).unwrap());
}

fn raise_sigint() {
    // SAFETY: raise(3) delivers to this thread; the handler is installed.
    assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
}
