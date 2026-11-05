//! Loopback-only fixture for capturing the real native app over SSH + SFTP.
#[path = "../tests/support/mod.rs"]
pub mod support;
use russh::keys::{
    agent::{AgentIdentity, client::AgentClient},
    ssh_key::HashAlg,
};
use std::{path::PathBuf, sync::Arc};
use yuuterm_core::Profile;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Supply a capture fixture output directory"))?,
    );
    let hold_seconds = std::env::args()
        .nth(2)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(180);
    anyhow::ensure!(
        (1..=3600).contains(&hold_seconds),
        "Fixture hold must be between 1 and 3600 seconds"
    );
    std::fs::create_dir_all(&dir)?;
    let mut agent = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent"),
    )
    .await??;
    let key = agent
        .request_identities()
        .await?
        .into_iter()
        .find_map(|k| match k {
            AgentIdentity::PublicKey { key, .. } => Some(key),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("No agent identity"))?;
    let fixture = support::Fixture::start(key.clone()).await;
    if std::env::var("YUUTERM_FIXTURE_LINKS").as_deref() == Ok("1") {
        support::add_links(&fixture.files);
    }
    if std::env::var("YUUTERM_FIXTURE_NAVIGATION").as_deref() == Ok("1") {
        support::add_navigation_tree(&fixture.files);
    }
    let mut profile = Profile {
        id: 1,
        name: "SSH protocol fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.identity_fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let engine = yuuterm_ssh::Engine::new(dir.join("host-keys.json"))?;
    let session = engine.connect(profile.clone(), Arc::new(|| {}))?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = session.snapshot();
            if s.trust.is_some() {
                session.trust(true);
            }
            if s.connected {
                break;
            }
            anyhow::ensure!(!s.finished, "{}", s.status);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    // Only an isolated generated loopback fixture is trusted automatically here.
    // The normal application never invokes this helper or approves keys itself.
    session.disconnect();
    std::fs::write(
        dir.join("profile.json"),
        serde_json::to_vec_pretty(&profile)?,
    )?;
    println!("Loopback capture fixture ready");
    if std::env::var("YUUTERM_FIXTURE_METRICS").as_deref() == Ok("1") {
        // Optional server-side counters for external-client qualification. No
        // keys, input/output bytes or real connection profiles are recorded.
        let end = tokio::time::Instant::now() + std::time::Duration::from_secs(hold_seconds);
        while tokio::time::Instant::now() < end {
            let counters = serde_json::json!({
                "active_connections": fixture.connections.load(std::sync::atomic::Ordering::SeqCst),
                "successful_authentications": fixture.auth.load(std::sync::atomic::Ordering::SeqCst),
            });
            std::fs::write(dir.join("metrics.json"), serde_json::to_vec(&counters)?)?;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    } else {
        tokio::time::sleep(std::time::Duration::from_secs(hold_seconds)).await;
    }
    Ok(())
}
