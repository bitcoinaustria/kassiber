fn main() {
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=LocalAuthentication");
    tauri_build::build();
}
