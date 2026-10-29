fn main() {
    let output =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg("--version")
            .output()
            .expect("rustc version");
    let version = String::from_utf8(output.stdout).expect("rustc version UTF-8");
    println!(
        "cargo:rustc-env=KASSIBER_DIAGNOSTICS_RUST_VERSION={}",
        version.trim()
    );
}
