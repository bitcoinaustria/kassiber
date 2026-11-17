use super::*;

// ---- Pure rules -----------------------------------------------------------------------------

#[test]
fn ready_must_come_first() {
    assert!(matches!(
        classify_line(
            r#"{"kind":"daemon.ready","schema_version":1,"data":{}}"#,
            false
        ),
        Line::Ready(_)
    ));
    match classify_line(r#"{"kind":"status","request_id":"a"}"#, false) {
        Line::Fatal(e) => assert_eq!(e.code, "daemon_protocol_error"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn post_ready_records_need_a_string_request_id() {
    assert!(matches!(
        classify_line(r#"{"kind":"status","request_id":"a"}"#, true),
        Line::Record { .. }
    ));
    // A second ready, or any other record without request_id, breaks the daemon.
    for line in [
        r#"{"kind":"daemon.ready","schema_version":1}"#,
        r#"{"kind":"status","schema_version":1}"#,
    ] {
        match classify_line(line, true) {
            Line::Fatal(e) => assert_eq!(e.code, "daemon_protocol_error"),
            other => panic!("{other:?}"),
        }
    }
    // A request_id that is not a string cannot match the active request.
    for line in [
        r#"{"kind":"status","request_id":7}"#,
        r#"{"kind":"status","request_id":null}"#,
    ] {
        match classify_line(line, true) {
            Line::Fatal(e) => assert_eq!(e.code, "daemon_request_id_mismatch"),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn event_records_carry_no_request_id_and_a_kind() {
    assert!(matches!(
        classify_line(
            r#"{"kind":"ui.freshness.background","schema_version":1,"event":true,"data":{}}"#,
            true
        ),
        Line::Event(_)
    ));
    for line in [
        r#"{"kind":"ui.freshness.background","event":true,"request_id":"a"}"#,
        r#"{"kind":"","event":true}"#,
        r#"{"event":true}"#,
    ] {
        assert!(
            matches!(classify_line(line, true), Line::Fatal(_)),
            "{line}"
        );
    }
}

#[test]
fn invalid_json_is_fatal() {
    assert!(
        matches!(classify_line("not json", true), Line::Fatal(e) if e.code == "daemon_protocol_error")
    );
}

#[test]
fn secret_store_requests_are_control_records() {
    assert!(matches!(
        classify_line(
            r#"{"kind":"supervisor.ai_secret_store.request","request_id":"s1","data":{}}"#,
            true
        ),
        Line::Control(_)
    ));
}

#[test]
fn terminal_records_are_exact_kind_error_or_auth_required() {
    assert_eq!(
        route_record("ui.overview.snapshot", false, "ui.overview.snapshot"),
        Route::Terminal
    );
    assert_eq!(
        route_record("ui.overview.snapshot", false, "error"),
        Route::Terminal
    );
    assert_eq!(
        route_record("ui.overview.snapshot", false, "auth_required"),
        Route::Terminal
    );
    assert_eq!(
        route_record("ui.freshness.run", true, "ui.freshness.progress"),
        Route::Progress
    );
    assert_eq!(
        route_record("ui.overview.snapshot", false, "progress"),
        Route::Ignore
    );
    assert!(
        matches!(route_record("ui.overview.snapshot", false, "ui.overview.delta"), Route::Violation(e) if e.code == "daemon_protocol_error")
    );
}

#[test]
fn error_envelope_shape() {
    let v = DaemonError::new("daemon_exited", "gone")
        .retryable()
        .hint("h")
        .with_stderr_tail("boom")
        .envelope("rune-1");
    assert_eq!(v["kind"], "error");
    assert_eq!(v["request_id"], "rune-1");
    assert_eq!(v["error"]["code"], "daemon_exited");
    assert_eq!(v["error"]["retryable"], true);
    assert_eq!(v["error"]["details"]["stderr_tail"], "boom");
}

#[test]
fn sensitive_details_are_redacted() {
    let d = DaemonError::new("x", "y")
        .details(json!({"passphrase_secret": "hunter2", "nested": {"api_key": "k"}, "fine": "ok"}))
        .details
        .unwrap();
    assert_eq!(d["passphrase_secret"], "[redacted]");
    assert_eq!(d["nested"]["api_key"], "[redacted]");
    assert_eq!(d["fine"], "ok");
}

#[test]
fn backoff_doubles_caps_and_resets() {
    let now = Instant::now();
    let mut b = Backoff::default();
    assert!(b.wait(now).is_none());
    b.fail(now);
    assert_eq!(b.wait(now), Some(Duration::from_secs(1)));
    b.fail(now);
    assert_eq!(b.wait(now), Some(Duration::from_secs(2)));
    for _ in 0..10 {
        b.fail(now);
    }
    assert_eq!(b.wait(now), Some(BACKOFF_MAX));
    assert!(b.wait(now + BACKOFF_MAX).is_none());
    b.reset();
    assert!(b.wait(now).is_none());
}

#[test]
fn lifecycle_levels() {
    let r = |event, detail: &str| LifecycleRecord {
        id: 1,
        ts_ms: 0,
        event,
        detail: detail.into(),
        stderr_tail: String::new(),
        source: "",
    };
    assert_eq!(r("spawned", "daemon ready").level(), "info");
    assert_eq!(r("exited", "exit status: 0").level(), "info");
    assert_eq!(r("exited", "exit status: 3").level(), "error");
    assert_eq!(r("exited", "exit status: 01").level(), "error");
    assert_eq!(r("killed", "daemon_protocol_error").level(), "error");
}

#[test]
fn state_root_precedence() {
    let tmp = std::env::temp_dir().join(format!("rune-state-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let home = tmp.as_path();
    let native = home.join("Library/Application Support/at.bitcoinaustria.kassiber");
    assert_eq!(
        state_root_for_platform("macos", Some(home), None, None),
        Some(native.clone())
    );
    // Absolute on every host (`/xdg` is not absolute on Windows).
    let xdg = home.join("xdg");
    assert_eq!(
        state_root_for_platform("linux", Some(home), Some(&xdg), None),
        Some(xdg.join("kassiber"))
    );
    assert_eq!(
        state_root_for_platform("linux", Some(home), Some(Path::new("relative")), None),
        Some(home.join(".local/share/kassiber"))
    );
    let lad = home.join("lad");
    assert_eq!(
        state_root_for_platform("windows", Some(home), None, Some(&lad)),
        Some(lad.join("at.bitcoinaustria.kassiber"))
    );
    // A legacy root holding only bin/run stays ignored; real state wins.
    std::fs::create_dir_all(home.join(".kassiber/bin")).unwrap();
    assert_eq!(
        state_root_for_platform("macos", Some(home), None, None),
        Some(native)
    );
    std::fs::create_dir_all(home.join(".kassiber/projects")).unwrap();
    assert_eq!(
        state_root_for_platform("macos", Some(home), None, None),
        Some(home.join(".kassiber"))
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn command_resolution_order() {
    let tmp = std::env::temp_dir().join(format!("rune-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let name = sidecar_filename().unwrap();
    let one_dir = tmp.join("res/binaries/kassiber-cli");
    std::fs::create_dir_all(&one_dir).unwrap();
    std::fs::write(one_dir.join(&name), b"").unwrap();
    let checkout = tmp.join("repo/target/release");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(
        checkout.join(format!("kassiber{}", std::env::consts::EXE_SUFFIX)),
        b"",
    )
    .unwrap();
    // A directory with the sidecar's name is skipped.
    std::fs::create_dir_all(tmp.join("dir/binaries/kassiber-cli").join(&name)).unwrap();

    let res = vec![tmp.join("res")];
    let env = resolve_command(Some("/opt/kassiber".into()), &res, Some(tmp.join("repo"))).unwrap();
    assert_eq!(
        (env.program.as_path(), env.source),
        (Path::new("/opt/kassiber"), "env_bin")
    );
    let bundled = resolve_command(None, &res, Some(tmp.join("repo"))).unwrap();
    assert_eq!(
        (bundled.program, bundled.source),
        (one_dir.join(&name), "bundled_sidecar")
    );
    let dev = resolve_command(None, &[tmp.join("dir")], Some(tmp.join("repo"))).unwrap();
    assert_eq!(dev.source, "checkout_build");
    assert_eq!(
        resolve_command(None, &[], None).unwrap_err().code,
        "daemon_binary_missing"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

// ---- A stub daemon --------------------------------------------------------------------------

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
/// A `sh` stand-in for `kassiber daemon`. Its mode is the `--data-root` argument:
/// - `normal` (or anything else): ready, then answers each request with its own kind and
///   `data.pid`, except `test.exit` (exits 3), `test.noid` (a record without request_id),
///   `test.delta` (a non-terminal record), `test.auth` (`auth_required`);
/// - `silent`: never sends ready; `crash`: dies at once after some stderr.
pub(crate) fn stub() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rune-stub-{}", std::process::id()));
    let path = dir.join("kassiber-stub.sh");
    if path.exists() {
        return path;
    }
    std::fs::create_dir_all(&dir).unwrap();
    let script = r#"#!/bin/sh
mode="$2"
case "$mode" in
  silent) sleep 10; exit 0 ;;
  crash) echo "boom passphrase_secret=hunter2" >&2; exit 1 ;;
esac
echo "starting" >&2
echo '{"kind":"daemon.ready","schema_version":1,"data":{"version":"stub"}}'
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  kind=$(printf '%s' "$line" | sed -n 's/.*"kind":"\([^"]*\)".*/\1/p')
  case "$kind" in
    test.exit) echo "dying on purpose" >&2; exit 3 ;;
    test.noid) echo '{"kind":"test.noid","schema_version":1}' ;;
    test.delta) echo "{\"kind\":\"test.delta.part\",\"schema_version\":1,\"request_id\":\"$id\"}" ;;
    test.auth) echo "{\"kind\":\"auth_required\",\"schema_version\":1,\"request_id\":\"$id\",\"data\":{\"scope\":\"unlock_database\"}}" ;;
    daemon.shutdown) echo "{\"kind\":\"daemon.shutdown\",\"schema_version\":1,\"request_id\":\"$id\"}"; exit 0 ;;
    *) echo "{\"kind\":\"$kind\",\"schema_version\":1,\"request_id\":\"$id\",\"data\":{\"pid\":$$}}" ;;
  esac
done
"#;
    let tmp = dir.join(format!(
        "kassiber-stub.{:?}.tmp",
        std::thread::current().id()
    ));
    std::fs::write(&tmp, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _ = std::fs::rename(&tmp, &path);
    path
}

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
fn supervisor(mode: &str) -> Supervisor {
    let launch = Launch {
        data_root: Some(mode.into()),
        explicit_root: true,
        bin_flag: Some(stub()),
        binary: stub(),
        env: Vec::new(),
    };
    let mut s = Supervisor::new(launch);
    // Generous: a loaded CI runner can take seconds to start `sh`. The silent stub's test sets
    // its own short timeout.
    s.ready_timeout = Duration::from_secs(10);
    s
}

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
/// Drains until `done` holds for what arrived, or 20 s pass.
fn drain_until(
    s: &mut Supervisor,
    done: impl Fn(&[(u64, Incoming)]) -> bool,
) -> Vec<(u64, Incoming)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut got = Vec::new();
    while Instant::now() < deadline {
        got.extend(s.drain());
        if done(&got) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
fn record_for<'a>(got: &'a [(u64, Incoming)], id: &str) -> Option<&'a Value> {
    got.iter().find_map(|(_, m)| match m {
        Incoming::Record {
            request_id,
            envelope,
        } if request_id == id => Some(envelope),
        _ => None,
    })
}

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
fn exited(got: &[(u64, Incoming)]) -> Option<&DaemonError> {
    got.iter().find_map(|(_, m)| {
        if let Incoming::Exited(e) = m {
            Some(e)
        } else {
            None
        }
    })
}

#[cfg(unix)]
#[test]
fn answers_requests_written_before_ready() {
    let mut s = supervisor("normal");
    let id = s.alloc_id();
    s.request(&id, "ui.overview.snapshot", &Value::Null)
        .unwrap();
    let got = drain_until(&mut s, |g| record_for(g, &id).is_some());
    assert!(got.iter().any(|(_, m)| matches!(m, Incoming::Ready(_))));
    assert_eq!(
        record_for(&got, &id).unwrap()["kind"],
        "ui.overview.snapshot"
    );
    assert!(s.lifecycle(0).0.iter().any(|r| r.event == "spawned"));
}

#[cfg(unix)]
#[test]
fn exit_fails_pending_and_the_next_request_respawns() {
    let mut s = supervisor("normal");
    let first = s.alloc_id();
    let g1 = s.request(&first, "status", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| record_for(g, &first).is_some());
    let pid1 = record_for(&got, &first).unwrap()["data"]["pid"]
        .as_i64()
        .unwrap();
    s.note_answer(g1);

    // The process dies mid-session: an exit, recorded with its stderr tail.
    let doomed = s.alloc_id();
    s.request(&doomed, "test.exit", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| exited(g).is_some());
    let err = exited(&got).expect("exit");
    assert_eq!(err.code, "daemon_exited");
    assert!(err.retryable);
    assert!(
        err.details.as_ref().unwrap()["stderr_tail"]
            .as_str()
            .unwrap()
            .contains("dying on purpose")
    );
    assert!(!s.running());
    let (ring, _) = s.lifecycle(0);
    assert!(
        ring.iter()
            .any(|r| r.event == "exited" && r.detail.contains('3')),
        "{ring:?}"
    );

    // Nothing restarts it by itself…
    std::thread::sleep(Duration::from_millis(100));
    assert!(s.drain().is_empty() && !s.running());
    // …the next request does, at once (it had served something, so no backoff).
    let next = s.alloc_id();
    let g2 = s.request(&next, "status", &Value::Null).unwrap();
    assert!(g2 > g1);
    let got = drain_until(&mut s, |g| record_for(g, &next).is_some());
    let pid2 = record_for(&got, &next).unwrap()["data"]["pid"]
        .as_i64()
        .unwrap();
    assert_ne!(pid1, pid2);
}

#[cfg(unix)]
#[test]
fn a_record_without_request_id_breaks_the_daemon() {
    let mut s = supervisor("normal");
    let id = s.alloc_id();
    s.request(&id, "test.noid", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| exited(g).is_some());
    let err = exited(&got).expect("broken");
    assert_eq!(err.code, "daemon_protocol_error");
    assert!(err.message.contains("without request_id"));
    assert!(s.lifecycle(0).0.iter().any(|r| r.event == "killed"));
    assert!(!s.running());
}

#[cfg(unix)]
#[test]
fn a_non_terminal_record_for_a_non_streaming_request_is_a_violation() {
    let mut s = supervisor("normal");
    let id = s.alloc_id();
    let g = s.request(&id, "test.delta", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| record_for(g, &id).is_some());
    let rec = record_for(&got, &id).unwrap();
    // The UI thread routes it (non-streaming kind) and breaks the process.
    let Route::Violation(err) = route_record("test.delta", false, rec["kind"].as_str().unwrap())
    else {
        panic!("not a violation")
    };
    let err = s.break_process(g, err);
    assert_eq!(err.code, "daemon_protocol_error");
    assert!(!s.running());
    assert!(
        s.lifecycle(0)
            .0
            .iter()
            .any(|r| r.event == "killed" && r.detail == "daemon_protocol_error")
    );
}

#[cfg(unix)]
#[test]
fn auth_required_is_terminal() {
    let mut s = supervisor("normal");
    let id = s.alloc_id();
    s.request(&id, "test.auth", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| record_for(g, &id).is_some());
    assert_eq!(
        route_record(
            "test.auth",
            false,
            record_for(&got, &id).unwrap()["kind"].as_str().unwrap()
        ),
        Route::Terminal
    );
}

#[cfg(unix)]
#[test]
fn no_ready_within_the_timeout_kills_and_backs_off() {
    let mut s = supervisor("silent");
    s.ready_timeout = Duration::from_millis(500);
    let id = s.alloc_id();
    s.request(&id, "status", &Value::Null).unwrap();
    let got = drain_until(&mut s, |g| exited(g).is_some());
    assert_eq!(exited(&got).unwrap().code, "daemon_timeout");
    assert!(s.lifecycle(0).0.iter().any(|r| r.event == "spawn_failed"));
    // A failed start delays the next one instead of looping.
    let again = s.alloc_id();
    assert_eq!(
        s.request(&again, "status", &Value::Null).unwrap_err().code,
        "daemon_unavailable"
    );
}

#[cfg(unix)]
#[test]
fn a_crash_on_start_is_recorded_redacted_and_backs_off() {
    let mut s = supervisor("crash");
    let id = s.alloc_id();
    s.request(&id, "status", &Value::Null).ok();
    let got = drain_until(&mut s, |g| exited(g).is_some());
    let err = exited(&got).expect("exit");
    let tail = err.details.as_ref().unwrap()["stderr_tail"]
        .as_str()
        .unwrap();
    assert!(tail.contains("boom") && !tail.contains("hunter2"), "{tail}");
    let (ring, _) = s.lifecycle(0);
    let rec = ring
        .iter()
        .find(|r| r.event == "spawn_failed")
        .expect("spawn_failed");
    assert!(!rec.stderr_tail.contains("hunter2"));
    assert!(s.backoff.failures >= 1);
}

#[cfg_attr(not(unix), allow(dead_code))] // used by the unix-only stub-daemon tests
/// Polls a switch to its outcome (the app does this from its pump, never blocking).
fn finish_switch(s: &mut Supervisor) -> Result<Option<u64>, DaemonError> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(r) = s.poll_switch() {
            return r;
        }
        assert!(Instant::now() < deadline, "switch never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn switch_root_warms_the_replacement_then_replaces() {
    let mut s = supervisor("normal");
    s.start().unwrap();
    let old_pid = s.pid().unwrap();
    // Same root and environment: nothing happens.
    assert!(!s.begin_switch(Some("normal".into()), Vec::new()).unwrap());
    assert!(s.poll_switch().is_none());
    // A root whose daemon never gets ready: the old one stays.
    s.ready_timeout = Duration::from_millis(400);
    assert!(s.begin_switch(Some("silent".into()), Vec::new()).unwrap());
    // Starting it returns at once: the caller is never blocked while it warms.
    assert!(s.switching());
    assert!(finish_switch(&mut s).is_err());
    assert!(!s.switching());
    assert_eq!(s.pid(), Some(old_pid));
    assert_eq!(s.data_root(), Some(Path::new("normal")));
    // Another working root (the stub treats any unknown mode as normal), with its own
    // environment: both apply only once it is ready.
    s.ready_timeout = Duration::from_secs(10);
    let env = vec![(
        "KASSIBER_OFFLINE_PREFERENCE_FILE".to_owned(),
        "/tmp/x".to_owned(),
    )];
    assert!(s.begin_switch(Some("other".into()), env.clone()).unwrap());
    assert_eq!(s.data_root(), Some(Path::new("normal")));
    assert!(s.launch.env.is_empty());
    let old = finish_switch(&mut s).unwrap();
    assert!(old.is_some());
    assert_ne!(s.pid(), Some(old_pid));
    assert_eq!(s.data_root(), Some(Path::new("other")));
    assert_eq!(s.launch.env, env);
    assert!(s.lifecycle(0).0.iter().any(|r| r.event == "replaced"));
    let id = s.alloc_id();
    s.request(&id, "status", &Value::Null).unwrap();
    assert!(record_for(&drain_until(&mut s, |g| record_for(g, &id).is_some()), &id).is_some());
    // Back to the base environment: a switch, even on the same root.
    assert!(s.begin_switch(Some("other".into()), Vec::new()).unwrap());
    finish_switch(&mut s).unwrap();
    assert!(s.launch.env.is_empty());
}

#[cfg(unix)]
#[test]
fn a_switch_with_no_process_running_starts_none_until_ready() {
    let mut s = supervisor("normal");
    assert!(s.pid().is_none());
    assert!(s.begin_switch(Some("other".into()), Vec::new()).unwrap());
    assert_eq!(finish_switch(&mut s).unwrap(), None);
    assert_eq!(s.data_root(), Some(Path::new("other")));
    assert!(s.pid().is_some());
}

/// The reader or watchdog thread marks the process broken, then kills it, then queues its exit.
/// A request sent in between retires the process before the exit is queued: the exit must still
/// come out of `drain`, exactly once, so every request the process had fails.
#[cfg(unix)]
#[test]
fn an_exit_racing_the_next_request_is_never_lost() {
    let mut s = supervisor("normal");
    let first = s.alloc_id();
    let g1 = s.request(&first, "status", &Value::Null).unwrap();
    drain_until(&mut s, |g| record_for(g, &first).is_some());
    // The window: broken, no exit queued yet.
    s.process
        .as_ref()
        .unwrap()
        .broken
        .store(true, Ordering::SeqCst);
    let next = s.alloc_id();
    let g2 = s.request(&next, "status", &Value::Null).unwrap();
    assert!(g2 > g1);
    let got = drain_until(&mut s, |g| record_for(g, &next).is_some());
    let exits: Vec<u64> = got
        .iter()
        .filter(|(_, m)| matches!(m, Incoming::Exited(_)))
        .map(|(g, _)| *g)
        .collect();
    assert_eq!(exits, vec![g1], "exactly one exit, for the retired process");
    // A late exit from the broken process's own thread reaches nobody: no second failure.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        s.drain()
            .iter()
            .all(|(_, m)| !matches!(m, Incoming::Exited(_)))
    );
}

#[test]
fn lifecycle_ring_caps_and_filters_by_cursor() {
    let mut s = Supervisor::new(Launch {
        data_root: Some("/nonexistent".into()),
        explicit_root: true,
        bin_flag: None,
        binary: "kassiber".into(),
        env: Vec::new(),
    });
    for _ in 0..70 {
        s.record("spawned", "daemon ready", "", "env_bin");
    }
    let (all, last) = s.lifecycle(0);
    assert_eq!(all.len(), LIFECYCLE_RING_CAPACITY);
    assert_eq!(last, 70);
    let (newer, _) = s.lifecycle(68);
    assert_eq!(newer.iter().map(|r| r.id).collect::<Vec<_>>(), vec![69, 70]);
}
