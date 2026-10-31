//! Explicitly invoked native X11 qualification against a real SSH host; never a
//! default test. Usage: x11_session <saved session name> <seconds> <command>.
//! Uses the saved profile with X11 forwarding switched on for this run only and a
//! copy of the pinned host keys, so the user's files are not changed. Run it from
//! a folder whose x11 subfolder holds the bundled runtime (target/release/examples).
use anyhow::{Context, Result, ensure};
use std::{sync::Arc, time::Duration};
use yuuterm_ssh::{Command, Engine, LiveSession};

fn text(live: &LiveSession) -> String {
    live.snapshot()
        .terminal
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect()
}
async fn wait(live: &LiveSession, f: impl Fn(&LiveSession) -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(40), async {
        loop {
            if f(live) {
                return Ok(());
            }
            let s = live.snapshot();
            ensure!(!s.finished, "Connection ended: {}", s.status);
            ensure!(s.trust.is_none(), "Host key is not pinned in YuuTerm");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("X11 session deadline")?
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let name = args.next().context("Supply a saved SSH session name")?;
    let seconds: u64 = args.next().context("Supply seconds")?.parse()?;
    let command = args.next().context("Supply the remote X command")?;
    let prefs = yuuterm_core::config::load().map_err(anyhow::Error::msg)?;
    let mut profile = prefs
        .profiles
        .into_iter()
        .find(|p| p.name == name)
        .context("Saved session not found")?;
    profile.ssh.x11_forward = true;
    profile.logging.enabled = false;
    let keys = yuuterm_core::config::path()?.with_file_name("host-keys.json");
    let dir = tempfile::tempdir()?;
    std::fs::copy(keys, dir.path().join("host-keys.json"))?;
    let engine = Engine::new(dir.path().join("host-keys.json"))?;
    let live = engine.connect(profile, Arc::new(|| {}))?;
    wait(&live, |l| l.snapshot().connected).await?;
    println!("connected; bundled X server ready");
    live.send(Command::Resize {
        rows: 30,
        cols: 120,
    })
    .map_err(anyhow::Error::msg)?;
    live.send(Command::Input(
        format!("echo YT_DISP${{DISPLAY:-none}}_END; ({command}) >/dev/null 2>&1 &\r").into_bytes(),
    ))
    .map_err(anyhow::Error::msg)?;
    // The last marker is the shell's output, not the echoed command line.
    let display = |l: &LiveSession| {
        let screen = text(l);
        let last = screen.rsplit("YT_DISP").next().unwrap_or_default();
        (!last.starts_with('$') && last.contains("_END"))
            .then(|| last.split("_END").next().unwrap_or_default().to_owned())
    };
    wait(&live, |l| display(l).is_some()).await?;
    let display = display(&live).unwrap_or_default();
    println!("remote DISPLAY={display}");
    ensure!(display != "none", "SSH server did not set DISPLAY");
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    let stats = live.x11_stats().context("No X11 forwarding")?;
    println!(
        "x11 channels: authenticated={} rejected={} active={}",
        stats.authenticated, stats.rejected, stats.active
    );
    live.disconnect();
    wait(&live, |l| l.snapshot().finished).await.ok();
    drop(live);
    println!("disconnected");
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}
