//! The Windows half of `tests/test_operator_protocol.py`: named-pipe
//! deadlines, saturation, the election and SID rejection. These compile and
//! run on Windows only; Linux CI cannot execute them.
#![cfg(windows)]

use kassiber_ipc::windows::current_sid;
use kassiber_ipc::windows::pipe::{connect, CreateError, PipeListener};
use kassiber_operator::protocol::{Channel, FrameChannel};
use kassiber_operator::runtime::{windows_pipe_name, PipeTransport, Runtime};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A pipe name of its own per test, so tests never meet the real broker.
fn name() -> String {
    format!(
        r"\\.\pipe\kassiber-operator-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

fn parse(text: &str) -> kassiber_pyjson::Value {
    kassiber_pytext::loads(text).unwrap()
}

#[test]
fn pipe_name_is_the_hashed_sid() {
    let name = windows_pipe_name("S-1-5-21-1-2-3-1001");
    assert!(name.starts_with(r"\\.\pipe\kassiber-operator-"));
    assert_eq!(name.len(), r"\\.\pipe\kassiber-operator-".len() + 24);
    let runtime = Runtime::from_process();
    assert_eq!(
        runtime.endpoint().unwrap(),
        kassiber_operator::runtime::Endpoint::Pipe(windows_pipe_name(&current_sid().unwrap()))
    );
}

#[test]
fn accepted_client_partial_frame_times_out() {
    let name = name();
    let sid = current_sid().unwrap();
    let listener = PipeListener::bind(&name, &sid).map_err(|_| ()).unwrap();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let accepted = listener
                .accept(&sid, Some(Duration::from_millis(50)))
                .unwrap();
            let mut channel = Channel::new(PipeTransport::from(accepted.stream), accepted.pid);
            channel.receive_json().unwrap_err()
        });
        let (mut client, _) =
            connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap();
        client.send_all(b"J").unwrap();
        let error = server.join().unwrap();
        assert!(error.is_timeout(), "{error:?}");
    });
}

#[test]
fn nonreading_client_does_not_block_server_write() {
    let name = name();
    let sid = current_sid().unwrap();
    let listener = PipeListener::bind(&name, &sid).map_err(|_| ()).unwrap();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let accepted = listener
                .accept(&sid, Some(Duration::from_millis(50)))
                .unwrap();
            let mut channel = Channel::new(PipeTransport::from(accepted.stream), accepted.pid);
            let payload = format!(r#"{{"payload": "{}"}}"#, "x".repeat(1024 * 1024));
            channel.send_json(&parse(&payload)).unwrap_err()
        });
        let _client = connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap();
        let error = server.join().unwrap();
        assert!(error.is_timeout(), "{error:?}");
    });
}

#[test]
fn listener_survives_more_than_32_connected_instances() {
    let name = name();
    let sid = current_sid().unwrap();
    let listener = PipeListener::bind(&name, &sid).map_err(|_| ()).unwrap();
    std::thread::scope(|scope| {
        let acceptor = scope.spawn(|| {
            (0..40)
                .map(|_| listener.accept(&sid, Some(Duration::from_secs(1))).unwrap())
                .collect::<Vec<_>>()
        });
        let clients: Vec<_> = (0..40)
            .map(|_| connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap())
            .collect();
        assert_eq!(acceptor.join().unwrap().len(), 40);
        let recovery = scope.spawn(|| {
            let accepted = listener.accept(&sid, Some(Duration::from_secs(1))).unwrap();
            let mut channel = Channel::new(PipeTransport::from(accepted.stream), accepted.pid);
            let request = channel.receive_json().unwrap();
            channel.send_json(&parse(r#"{"ok": true}"#)).unwrap();
            request
        });
        let (stream, _) =
            connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap();
        let mut channel = Channel::new(PipeTransport::from(stream), None);
        channel
            .send_json(&parse(r#"{"after_saturation": true}"#))
            .unwrap();
        assert_eq!(channel.receive_json().unwrap(), parse(r#"{"ok": true}"#));
        assert_eq!(
            recovery.join().unwrap(),
            parse(r#"{"after_saturation": true}"#)
        );
        drop(clients);
    });
}

#[test]
fn second_listener_loses_election_without_disturbing_first() {
    let name = name();
    let sid = current_sid().unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results: Vec<_> = std::thread::scope(|scope| {
        let starters: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    PipeListener::bind(&name, &sid)
                })
            })
            .collect();
        starters.into_iter().map(|s| s.join().unwrap()).collect()
    });
    let mut winners = Vec::new();
    let mut losers = 0;
    for result in results {
        match result {
            Ok(listener) => winners.push(listener),
            Err(CreateError::AlreadyRunning) => losers += 1,
            Err(CreateError::Io(error)) => panic!("{error}"),
        }
    }
    assert_eq!((winners.len(), losers), (1, 1));
    let listener = &winners[0];
    std::thread::scope(|scope| {
        let server = scope.spawn(|| {
            let accepted = listener.accept(&sid, Some(Duration::from_secs(1))).unwrap();
            let mut channel = Channel::new(PipeTransport::from(accepted.stream), accepted.pid);
            let request = channel.receive_json().unwrap();
            channel.send_json(&parse(r#"{"winner": true}"#)).unwrap();
            request
        });
        let (stream, server_sid) =
            connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap();
        assert_eq!(server_sid, sid);
        let mut channel = Channel::new(PipeTransport::from(stream), None);
        channel
            .send_json(&parse(r#"{"still_running": true}"#))
            .unwrap();
        assert_eq!(
            channel.receive_json().unwrap(),
            parse(r#"{"winner": true}"#)
        );
        assert_eq!(server.join().unwrap(), parse(r#"{"still_running": true}"#));
    });
}

#[test]
fn accepted_clients_report_this_sid_and_pid() {
    let name = name();
    let sid = current_sid().unwrap();
    let listener = PipeListener::bind(&name, &sid).map_err(|_| ()).unwrap();
    std::thread::scope(|scope| {
        let server = scope.spawn(|| listener.accept(&sid, Some(Duration::from_secs(1))).unwrap());
        let _client = connect(&name, Duration::from_secs(5), Some(Duration::from_secs(1))).unwrap();
        let accepted = server.join().unwrap();
        assert_eq!(accepted.sid, sid);
        assert_eq!(accepted.pid, Some(std::process::id()));
        // A rejected client sees the pipe close, not a timeout.
        accepted.stream.reject();
    });
}

#[test]
fn listener_close_ends_a_pending_accept_once() {
    let name = name();
    let sid = current_sid().unwrap();
    let listener = PipeListener::bind(&name, &sid).map_err(|_| ()).unwrap();
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| listener.accept(&sid, None).map(|_| ()));
        std::thread::sleep(Duration::from_millis(100));
        listener.close();
        let error = waiting.join().unwrap().unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::TimedOut);
    });
    listener.close();
}
