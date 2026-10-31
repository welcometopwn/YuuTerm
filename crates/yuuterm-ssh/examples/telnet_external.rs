//! Explicitly invoked external TELNET qualification against a real telnetd and
//! OS login; never a default test. Host/account/password come from the runner's
//! environment (YUUTERM_TELNET_TEST_HOST/_USER/_PASSWORD) and are not logged.
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, sync::Arc, time::Duration};
use yuuterm_core::{Profile, Protocol};
use yuuterm_ssh::{Command, Engine, LiveSession};

async fn wait(
    live: &LiveSession,
    label: &str,
    f: impl Fn(&yuuterm_ssh::Snapshot) -> bool,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = live.snapshot();
            if f(&s) {
                return Ok(());
            }
            ensure!(!s.finished, "Connection ended: {}", s.status);
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .with_context(|| format!("TELNET deadline: {label}"))?
}
fn text(live: &LiveSession) -> String {
    live.snapshot()
        .terminal
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect()
}
fn input(live: &LiveSession, bytes: &[u8]) -> Result<()> {
    live.send(Command::Input(bytes.to_vec()))
        .map_err(anyhow::Error::msg)
}
#[tokio::main]
async fn main() -> Result<()> {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Supply the runner directory")?,
    );
    let var = |name: &str| std::env::var(name).with_context(|| format!("{name} missing"));
    let (host, user, password) = (
        var("YUUTERM_TELNET_TEST_HOST")?,
        var("YUUTERM_TELNET_TEST_USER")?,
        var("YUUTERM_TELNET_TEST_PASSWORD")?,
    );
    let profile = Profile {
        id: 1,
        name: "External TELNET qualification".into(),
        protocol: Protocol::Telnet,
        host,
        port: 23,
        username: user.clone(),
        ..Default::default()
    };
    let engine = Engine::new(folder.join("host-keys.json"))?;
    let live = engine.connect(profile, Arc::new(|| {}))?;
    let mut checks = Vec::<String>::new();
    let result: Result<()> = async {
        wait(&live, "connected", |s| s.connected).await?;
        live.send(Command::Resize { rows: 30, cols: 100 }).map_err(anyhow::Error::msg)?;
        wait(&live, "login prompt", |_| text(&live).contains("login:")).await?;
        checks.push("Real telnetd negotiates and presents the OS login prompt".into());
        input(&live, format!("{user}\r").as_bytes())?;
        wait(&live, "password prompt", |_| text(&live).contains("Password:")).await?;
        input(&live, format!("{password}\r").as_bytes())?;
        // login(1) flushes typeahead; wait for the shell prompt before typing.
        wait(&live, "shell prompt", |_| {
            text(&live)
                .rsplit("Password:")
                .next()
                .is_some_and(|after| after.contains("$ "))
        })
        .await?;
        input(&live, b"unset HISTFILE; PS1='yt$ '; printf 'YT_%s\\n' LOGIN_OK\r")?;
        wait(&live, "login", |_| text(&live).contains("YT_LOGIN_OK")).await?;
        ensure!(!text(&live).contains(&password), "Password was echoed");
        checks.push("OS login succeeds; the password is not echoed".into());
        input(&live, b"printf 'YT_TERM_%s\\n' \"$TERM\"; echo YT_SIZE_$(stty size | tr ' ' x)\r")?;
        wait(&live, "size", |_| text(&live).contains("YT_SIZE_30x100") && text(&live).contains("YT_TERM_xterm")).await?;
        checks.push("TTYPE and NAWS reach the server PTY (TERM xterm*, stty 30x100)".into());
        live.send(Command::Resize { rows: 40, cols: 120 }).map_err(anyhow::Error::msg)?;
        input(&live, b"echo YT_RESIZED_$(stty size | tr ' ' x)\r")?;
        wait(&live, "resize", |_| text(&live).contains("YT_RESIZED_40x120")).await?;
        checks.push("Live resize renegotiates NAWS to 40x120".into());
        input(&live, "printf '\\316\\251 %s\\n' \"$(printf '\\346\\227\\245')\"; echo 'typed Ω'\r".as_bytes())?;
        wait(&live, "unicode", |_| text(&live).contains("Ω 日") && text(&live).contains("typed Ω")).await?;
        checks.push("UTF-8 output and typed UTF-8 input pass through unchanged".into());
        // Client to server: a typed 0xFF must arrive as one byte (IAC IAC on the wire).
        input(&live, b"stty raw -echo; head -c 3 | od -An -tx1 | tr -d ' ' | sed 's/^/YT_IN_/'; stty sane\r")?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        input(&live, &[0xff, b'A', 0xff])?;
        wait(&live, "iac input", |_| text(&live).contains("YT_IN_ff41ff")).await?;
        // Server to client: telnetd sends each 0xFF as IAC IAC. The headless vt100
        // view drops invalid UTF-8, so check that the bytes after each 0xFF are
        // not consumed as TELNET commands (the stream stays in sync).
        input(&live, b"printf 'YT_OUT_%b_END\\n' '\\377A\\377\\373B'\r")?;
        wait(&live, "iac output", |_| text(&live).contains("YT_OUT_AB_END")).await?;
        checks.push("Byte 0xFF passes client to server as one byte; server-sent 0xFF (incl. 0xFF 0xFB) never swallows following data".into());
        input(&live, b"sleep 30; printf 'YT_%s\\n' NOT_INTERRUPTED\r")?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        input(&live, b"\x03")?;
        input(&live, b"printf 'YT_%s\\n' INTERRUPTED\r")?;
        wait(&live, "interrupt", |_| text(&live).contains("YT_INTERRUPTED")).await?;
        ensure!(!text(&live).contains("YT_NOT_INTERRUPTED"), "Ctrl+C did not interrupt");
        checks.push("Ctrl+C interrupts a running remote command".into());
        input(&live, b"seq 1 20000 | tail -n 1 | sed 's/^/YT_FLOOD_/'\r")?;
        input(&live, b"seq 1 20000\r")?;
        input(&live, b"printf 'YT_%s\\n' FLOOD_DONE\r")?;
        wait(&live, "flood", |_| text(&live).contains("YT_FLOOD_DONE")).await?;
        checks.push("20,000-line output burst completes and the session stays responsive".into());
        input(&live, b"exit\r")?;
        tokio::time::timeout(Duration::from_secs(15), async {
            while !live.snapshot().finished {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .context("Session did not end after exit")?;
        checks.push("Remote logout ends the session cleanly".into());
        Ok(())
    }
    .await;
    live.disconnect();
    std::fs::write(
        folder.join("telnet-report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed": result.is_ok(),
            "checks": checks,
            "error": result.as_ref().err().map(|e| format!("{e:#}")),
            // Failure only: the test account's screen, kept in the runner folder.
            "screen": result.is_err().then(|| text(&live).trim_end().to_owned()),
        }))?,
    )?;
    result
}
