fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_ACTIONS");
    tauri_build::build()
}
