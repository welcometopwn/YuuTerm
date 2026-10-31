//! Read-only installed-server check; prints readiness only, never authority data.
#[tokio::main]
async fn main() {
    println!(
        "{}",
        serde_json::to_string(&yuuterm_ssh::x11::desktop::probe().await).unwrap()
    );
}
