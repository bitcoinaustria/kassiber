use kassiber_envelope::{get, s};
use kassiber_setup::core_credentials::{candidates, reference, resolve};
#[test]
fn core_references_reselect_current_local_credentials_without_password_in_identity() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(".bitcoin");
    std::fs::create_dir(&dir).unwrap();
    let conf = dir.join("bitcoin.conf");
    std::fs::write(&conf,"rpcuser=alice\nrpcpassword=synthetic-old\n[main]\nrpcport=18444\n[test]\nrpcconnect=example.test\n").unwrap();
    let found = candidates(root.path()).unwrap();
    assert_eq!(found.len(), 3);
    assert_eq!(get(&found[0], "url"), Some(&s("http://127.0.0.1:18444")));
    assert_eq!(reference(&found[0]), "local-core:6f4c276aca76e7d5258e05f0");
    let id = reference(&found[0]);
    std::fs::write(
        &conf,
        "rpcuser=alice\nrpcpassword=synthetic-new\nmain.rpcport=18444\n",
    )
    .unwrap();
    assert_eq!(
        get(&resolve(root.path(), &id).unwrap(), "password"),
        Some(&s("synthetic-new"))
    );
    std::fs::write(
        &conf,
        "rpcuser=bob\nrpcpassword=synthetic-new\nmain.rpcport=18444\n",
    )
    .unwrap();
    assert_eq!(resolve(root.path(), &id).unwrap_err().code, "validation");
    std::fs::write(
        &conf,
        "rpcuser=alice\nrpcpassword=synthetic-new\nrpcconnect=example.test\n",
    )
    .unwrap();
    assert!(candidates(root.path()).unwrap().is_empty());
    std::fs::write(
        &conf,
        "rpcuser=alice\nrpcpassword=synthetic-new\nrpcurl=http://127.0.0.1%invalid:8332\n",
    )
    .unwrap();
    assert!(candidates(root.path()).unwrap().is_empty());
}

#[test]
fn core_candidate_alias_precedence_cookie_dedup_and_ipv6_match_python() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(".bitcoin");
    std::fs::create_dir_all(dir.join("testnet3")).unwrap();
    std::fs::create_dir_all(dir.join("testnet4")).unwrap();
    std::fs::write(dir.join("testnet3/.cookie"), "synthetic-cookie").unwrap();
    std::fs::write(dir.join("testnet4/.cookie"), "synthetic-cookie").unwrap();
    std::fs::write(dir.join("bitcoin.conf"),"rpcuser=alice\nrpcpassword=synthetic-secret\n[main]\nrpcconnect=[::1]:18445\n[test]\nrpcport=18333\n[testnet4]\nrpcport=18334\n").unwrap();
    let found = candidates(root.path()).unwrap();
    assert_eq!(found.len(), 6);
    assert_eq!(get(&found[0], "url"), Some(&s("http://[::1]:18445")));
    assert_eq!(get(&found[1], "url"), Some(&s("http://127.0.0.1:18334")));
    assert_eq!(get(&found[2], "auth_source"), Some(&s("cookiefile")));
    assert_eq!(get(&found[3], "auth_source"), Some(&s("cookiefile")));
}
