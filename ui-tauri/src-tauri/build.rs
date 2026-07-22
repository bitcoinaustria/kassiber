fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_CHANNEL");
    let build_channel = std::env::var("BUILD_CHANNEL").unwrap_or_else(|_| "dev".to_string());
    assert!(
        matches!(build_channel.as_str(), "dev" | "prerelease" | "release"),
        "BUILD_CHANNEL must be dev, prerelease, or release"
    );
    println!("cargo:rustc-env=KASSIBER_BUILD_CHANNEL={build_channel}");
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=LocalAuthentication");
    tauri_build::build();
}
