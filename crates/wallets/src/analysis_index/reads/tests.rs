use super::*;
use kassiber_envelope::AppError;

fn compare(prefix: &str) {
    compare_in(
        include_str!("../../../tests/data/chain_analysis_reads.json"),
        prefix,
    );
}
fn compare_psbt(prefix: &str) {
    compare_in(
        include_str!("../../../tests/data/chain_analysis_psbt_reads.json"),
        prefix,
    );
}

fn compare_in(data: &str, prefix: &str) {
    let cases = kassiber_pytext::loads(data).unwrap();
    let mut checked = 0;
    for case in list(&cases)
        .iter()
        .filter(|c| text(c, "name").starts_with(prefix))
    {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        let result = fixture::evaluate(case, &conn).unwrap();
        let expected = list(field(case, "expected"));
        assert_eq!(list(&result).len(), expected.len());
        for (step, (actual, expected)) in list(&result).iter().zip(expected).enumerate() {
            let a = text(actual, "wire");
            let b = text(expected, "wire");
            if a != b {
                let at = a
                    .bytes()
                    .zip(b.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(a.len().min(b.len()));
                panic!(
                    "{} step {step} envelope byte {at}: native {:?}, Python {:?}",
                    text(case, "name"),
                    a.get(at.saturating_sub(50)..(at + 160).min(a.len())),
                    b.get(at.saturating_sub(50)..(at + 160).min(b.len()))
                );
            }
            assert_eq!(
                json(field(actual, "state")).unwrap(),
                json(field(expected, "state")).unwrap(),
                "{} step {step} all tables",
                text(case, "name")
            );
            assert_eq!(
                field(actual, "in_transaction"),
                field(expected, "in_transaction"),
                "{} step {step} transaction",
                text(case, "name")
            );
        }
        assert!(conn.is_autocommit());
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn chain_analysis_query_retains_generation_observer_scope_and_rollback() {
    compare("query");
}
#[test]
fn chain_analysis_entropy_preserves_validation_and_physical_subjects() {
    compare("entropy");
}
#[test]
fn chain_analysis_cases_verify_integrity_and_compare_all_surfaces() {
    compare("cases");
}
#[test]
fn chain_analysis_datasets_bind_cursors_state_and_visibility() {
    compare("datasets");
}
#[test]
fn chain_analysis_labels_remain_ordered_and_book_scoped() {
    compare("labels");
}

#[test]
fn chain_analysis_psbt_analysis_keeps_reference_order_and_validation() {
    compare_psbt("analyze");
}
#[test]
fn chain_analysis_psbt_entropy_binds_snapshot_and_limits() {
    compare_psbt("entropy");
}
#[test]
fn chain_analysis_psbt_compare_reports_delta_and_constraints() {
    compare_psbt("compare");
}
#[test]
fn chain_analysis_payjoin_checks_verify_commitments_and_outputs() {
    compare_psbt("payjoin");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-scratch/chain-analysis-runtime")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn code(result: Result<Value>) -> String {
    result.unwrap_err().code
}
fn wait(runtime: &Runtime, scope: &[u8], id: &Value) -> Value {
    for _ in 0..2000 {
        let receipt = runtime.get(scope, id, false).unwrap();
        if field(&receipt, "status") != &s("running") {
            return receipt;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("job did not finish");
}

#[test]
fn chain_analysis_sources_pin_purpose_book_and_identity() {
    let dir = scratch("sources");
    let conn = Connection::open_in_memory().unwrap();
    let runtime = Runtime::new();
    let file = dir.join("a.psbt");
    std::fs::write(&file, b"psbt").unwrap();
    let scope = runtime::scope_key(&conn, "p");
    let path = s(file.to_str().unwrap());
    assert_eq!(
        code(runtime.stage(&conn, scope.clone(), &path, &s("other"))),
        "validation"
    );
    std::fs::write(dir.join("empty"), b"").unwrap();
    for bad in [
        s(dir.join("empty").to_str().unwrap()),
        s(dir.to_str().unwrap()),
        s(dir.join("missing").to_str().unwrap()),
        Value::Int(1.into()),
    ] {
        assert_eq!(
            code(runtime.stage(&conn, scope.clone(), &bad, &s("psbt"))),
            "chain_analysis_source_unavailable"
        );
    }
    let staged = runtime
        .stage(&conn, scope.clone(), &path, &s("psbt"))
        .unwrap();
    assert_eq!(field(&staged, "filename"), &s("a.psbt"));
    assert_eq!(field(&staged, "size_bytes"), &Value::Int(4.into()));
    let token = field(&staged, "source_token").clone();
    assert_eq!(token.as_str().unwrap().len(), 43);
    let mut source = runtime.open(&scope, &token, "psbt").unwrap();
    assert_eq!(source.read_up_to(3).unwrap(), b"psb");
    let other = runtime::scope_key(&conn, "q");
    for (scope, purpose) in [(&other, "psbt"), (&scope, "dataset")] {
        assert_eq!(
            runtime.open(scope, &token, purpose).err().unwrap().code,
            "chain_analysis_source_expired"
        );
    }
    std::fs::write(&file, b"psbt changed").unwrap();
    assert_eq!(
        source.read_up_to(10).err().unwrap().code,
        "chain_analysis_source_changed"
    );
    assert_eq!(
        runtime.open(&scope, &token, "psbt").err().unwrap().code,
        "chain_analysis_source_changed"
    );
    runtime.clear();
    assert_eq!(
        runtime.open(&scope, &token, "psbt").err().unwrap().code,
        "chain_analysis_source_expired"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn chain_analysis_jobs_fence_scope_capacity_and_cancellation() {
    let conn = Connection::open_in_memory().unwrap();
    let runtime = Runtime::new();
    let scope = runtime::scope_key(&conn, "p");
    let gate = Arc::new(std::sync::Barrier::new(2));
    let blocked = |gate: Arc<std::sync::Barrier>| -> runtime::Compute {
        Box::new(move |worker| {
            worker.progress(dict([("phase", s("waiting"))]));
            gate.wait();
            while !worker.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(AppError::new("validation", "stopped"))
        })
    };
    let first = runtime
        .start(
            &conn,
            scope.clone(),
            dict([("n", Value::Int(1.into()))]),
            blocked(Arc::clone(&gate)),
        )
        .unwrap();
    gate.wait();
    let id = field(&first, "job_id").clone();
    let running = runtime.get(&scope, &id, false).unwrap();
    assert_eq!(field(&running, "status"), &s("running"));
    assert_eq!(
        field(&field(&running, "progress").clone(), "phase"),
        &s("waiting")
    );
    assert_eq!(
        field(&running, "request"),
        &dict([("n", Value::Int(1.into()))])
    );
    assert_eq!(
        code(runtime.get(&runtime::scope_key(&conn, "q"), &id, false)),
        "chain_analysis_job_unavailable"
    );
    assert_eq!(
        code(runtime.get(&scope, &Value::Int(1.into()), false)),
        "chain_analysis_job_unavailable"
    );
    let gate2 = Arc::new(std::sync::Barrier::new(2));
    let second = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            blocked(Arc::clone(&gate2)),
        )
        .unwrap();
    gate2.wait();
    let busy = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| Ok(dict::<&str>([]))),
        )
        .unwrap_err();
    assert_eq!(
        (busy.code.as_str(), busy.retryable),
        ("chain_analysis_busy", true)
    );
    let cancelled = runtime.get(&scope, &id, true).unwrap();
    assert_eq!(field(&cancelled, "cancel_requested"), &Value::Bool(true));
    let done = wait(&runtime, &scope, &id);
    assert_eq!(field(&done, "status"), &s("cancelled"));
    assert_eq!(field(&done, "error_code"), &Value::Null);
    let ok = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| Ok(dict([("value", s("x"))]))),
        )
        .unwrap();
    let ok = wait(&runtime, &scope, field(&ok, "job_id"));
    assert_eq!(field(&ok, "status"), &s("completed"));
    assert_eq!(field(&ok, "result"), &dict([("value", s("x"))]));
    let failed = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| Err(AppError::new("chain_analysis_invalid_psbt", "x"))),
        )
        .unwrap();
    let failed = wait(&runtime, &scope, field(&failed, "job_id"));
    assert_eq!(
        (field(&failed, "status"), field(&failed, "error_code")),
        (&s("failed"), &s("chain_analysis_invalid_psbt"))
    );
    let panicked = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| panic!("hidden")),
        )
        .unwrap();
    let panicked = wait(&runtime, &scope, field(&panicked, "job_id"));
    assert_eq!(
        field(&panicked, "error_code"),
        &s("chain_analysis_computation_failed")
    );
    // Quiescing cancels the remaining worker and closes admission.
    let guard = runtime.quiesce(std::time::Duration::from_secs(5)).unwrap();
    let blocked_start = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| Ok(dict::<&str>([]))),
        )
        .unwrap_err();
    assert_eq!(blocked_start.code, "chain_analysis_busy");
    assert_eq!(
        field(
            &runtime
                .get(&scope, field(&second, "job_id"), false)
                .unwrap(),
            "status"
        ),
        &s("cancelled")
    );
    drop(guard);
    runtime.clear();
    assert_eq!(
        code(runtime.get(&scope, &id, false)),
        "chain_analysis_job_unavailable"
    );
    for _ in 0..16 {
        let job = runtime
            .start(
                &conn,
                scope.clone(),
                dict::<&str>([]),
                Box::new(|_| Ok(dict::<&str>([]))),
            )
            .unwrap();
        wait(&runtime, &scope, field(&job, "job_id"));
    }
    let next = runtime
        .start(
            &conn,
            scope.clone(),
            dict::<&str>([]),
            Box::new(|_| Ok(dict::<&str>([]))),
        )
        .unwrap();
    wait(&runtime, &scope, field(&next, "job_id"));
}
