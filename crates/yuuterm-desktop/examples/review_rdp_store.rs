//! Loopback-free witness: app-produced RDP pin metadata must survive its reader.
#[path = "../src/rdp_cert.rs"]
pub mod rdp_cert;
fn main() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/security-review-tests");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let path = dir.path().join("pins.json");
    let fingerprint = "a".repeat(64);
    let host = |i| {
        format!(
            "{i:04}.{}.{}.{}.{}.test",
            "a".repeat(59),
            "b".repeat(59),
            "c".repeat(59),
            "d".repeat(59)
        )
    };
    for i in 0..900 {
        rdp_cert::remember(&path, &host(i), 3389, &fingerprint).unwrap();
    }
    let first = rdp_cert::remembered(&path, &host(0), 3389, &fingerprint);
    let last = rdp_cert::remembered(&path, &host(899), 3389, &fingerprint);
    let entries: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    println!(
        "{}",
        serde_json::json!({"fixture":"900 synthetic valid-length hostnames, no network or credentials", "requested_pins":900, "retained_pins":entries.as_object().unwrap().len(), "first_remembered":first, "last_remembered":last})
    );
    if !first || !last {
        std::process::exit(2);
    }
}
