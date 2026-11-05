//! Explicitly invoked external Linux OpenSSH qualification; never a default test.
//! Reads an isolated key-file profile/pin. Mutates only its generated test run.
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use yuuterm_core::{Profile, SshAuth};
use yuuterm_ssh::{Command, Engine, LiveSession, files::FileAction};

async fn wait(live: &LiveSession, f: impl Fn(&yuuterm_ssh::Snapshot) -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = live.snapshot();
            if f(&s) {
                return Ok(());
            }
            ensure!(!s.finished, "Connection ended: {}", s.status);
            ensure!(
                s.trust.is_none(),
                "Test host is not pinned; verify the runner's isolated host key"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .context("Linux OpenSSH deadline")?
}
fn text(live: &LiveSession) -> String {
    live.snapshot()
        .terminal
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect()
}
#[tokio::main]
async fn main() -> Result<()> {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Supply the isolated runner directory")?,
    );
    let assets = folder.join(format!("engine-assets-{:x}", rand::random::<u64>()));
    std::fs::create_dir(&assets)?;
    let profile: Profile = serde_json::from_slice(&std::fs::read(folder.join("profile.json"))?)?;
    ensure!(
        profile.ssh.auth == SshAuth::KeyFile && !profile.logging.enabled,
        "Key-file fixture with logging off required"
    );
    let remote = std::fs::read_to_string(folder.join("remote-path.txt"))?;
    let prefix = format!("/home/{}/yuuterm-test/run-", profile.username);
    ensure!(
        remote.starts_with(&prefix)
            && remote[prefix.len()..].len() == 32
            && remote[prefix.len()..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Remote writes must stay in the generated test subtree"
    );
    let engine = Engine::new(folder.join("host-keys.json"))?;
    // Optional SSH gateways: manual key-file hops from the runner.
    let hops = profile
        .gateways
        .iter()
        .map(|g| match g {
            yuuterm_core::Gateway::Manual(hop) => Ok(yuuterm_ssh::Hop {
                gateway: hop.clone(),
                password: None,
            }),
            yuuterm_core::Gateway::Saved { .. } => anyhow::bail!("Manual gateways only"),
        })
        .collect::<Result<Vec<_>>>()?;
    let through = hops.len();
    let live = engine.connect_via(profile, hops, Arc::new(|| {}), None)?;
    wait(&live, |s| s.connected).await?;
    let mut checks = vec!["Key-file SSH authentication with the isolated host pin".to_owned()];
    if through > 0 {
        checks.push(format!(
            "Connected through {through} SSH gateway(s), each verified against its pinned host key"
        ));
    }
    let result:Result<()>=async {
        live.send(Command::Resize{rows:30,cols:100}).map_err(anyhow::Error::msg)?;
        // Bash builds the markers from pieces so the echoed command line cannot match.
        live.send(Command::Input(b"unset HISTFILE; printf 'YT_%s\\n' SHELL_OK; printf '\\316\\251\\n'; echo YT_SIZE_$(stty size | tr ' ' x)\r".to_vec())).map_err(anyhow::Error::msg)?;
        wait(&live, |_|text(&live).contains("YT_SHELL_OK") && text(&live).contains('\u{03a9}')).await?;
        checks.push("Actual bash PTY output and Unicode Omega rendered by the engine terminal".into());
        wait(&live, |_|text(&live).contains("YT_SIZE_30x100")).await?;
        checks.push("PTY resize independently observed as 30x100 by stty".into());
        let files=live.files();
        let cancel=CancellationToken::new();
        let done=files.execute(FileAction::Resolve{path:remote.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(done.resolved.context("No resolved path")?.directory,"Test run must be a directory");
        live.send(Command::Browse(remote.clone())).map_err(anyhow::Error::msg)?;
        wait(&live,|s|s.path==remote).await?;
        checks.push("SFTP resolves and browses the generated Linux directory".into());
        let path=format!("{remote}/roundtrip.bin");
        let original=(0..65539).map(|i|(i%251)as u8).collect::<Vec<_>>();
        let local=assets.join("roundtrip.bin");tokio::fs::write(&local,&original).await?;
        files.execute(FileAction::Upload{path:path.clone(),local:local.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let download=assets.join("download.bin");
        files.execute(FileAction::Download{path:path.clone(),local:download.clone(),edit:false},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(tokio::fs::read(download).await?==original,"Binary roundtrip mismatch");
        checks.push("SFTP upload/download preserve 65,539 binary bytes exactly".into());
        let directory=format!("{remote}/tree");
        files.execute(FileAction::Mkdir{path:directory.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        files.execute(FileAction::Create{path:format!("{directory}/empty.txt")},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let unicode=assets.join("unicode.txt");let content="LF\nCRLF\r\nUnicode \u{03a9} \u{65e5}\n";tokio::fs::write(&unicode,content).await?;
        files.execute(FileAction::Upload{path:format!("{directory}/space \u{03a9}.txt"),local:unicode.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        // A server-side symlink: the tree download must skip it, resolve must follow it.
        live.send(Command::Input(format!("ln -s 'space \u{03a9}.txt' '{directory}/link.txt' && printf 'YT_%s\\n' LINK_OK\r").into_bytes())).map_err(anyhow::Error::msg)?;
        wait(&live, |_|text(&live).contains("YT_LINK_OK")).await?;
        let tree=assets.join("tree-download");
        let downloaded=files.execute(FileAction::DownloadTree{path:directory.clone(),local:tree.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(tokio::fs::read(tree.join("space \u{03a9}.txt")).await?==content.as_bytes(),"Directory Unicode/line-ending mismatch");
        ensure!(tokio::fs::metadata(tree.join("empty.txt")).await?.len()==0,"New file is not empty");
        ensure!(downloaded.skipped==1 && !tree.join("link.txt").exists(),"Symlink was not skipped by the tree download");
        checks.push("New file/folder and recursive directory download preserve Unicode names and LF/CRLF bytes and skip the symlink".into());
        let link=files.execute(FileAction::Properties{path:format!("{directory}/link.txt")},cancel.clone()).await.map_err(anyhow::Error::msg)?.properties.context("No link properties")?;
        let target=files.execute(FileAction::Resolve{path:format!("{directory}/link.txt")},cancel.clone()).await.map_err(anyhow::Error::msg)?.resolved.context("No link target")?;
        ensure!(link.symlink && target.path==format!("{directory}/space \u{03a9}.txt") && !target.directory,"Symlink not reported or resolved");
        checks.push("SFTP reports the symlink and resolves it to its Unicode target".into());
        files.execute(FileAction::Rename{path:path.clone(),name:"renamed ' \u{03a9}.bin".into()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let renamed=format!("{remote}/renamed ' \u{03a9}.bin");
        files.execute(FileAction::Permissions{path:renamed.clone(),mode:0o640,recursive:false},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let mode=files.execute(FileAction::Properties{path:renamed.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?.properties.context("No properties")?;
        ensure!(!mode.windows_permissions && mode.permissions.map(|p|p&0o7777)==Some(0o640),"POSIX mode not applied");
        // Double quotes: the name contains an apostrophe.
        live.send(Command::Input(format!("printf 'YT_MODE_%s\\n' $(stat -c %a \"{renamed}\")\r").into_bytes())).map_err(anyhow::Error::msg)?;
        wait(&live, |_|text(&live).contains("YT_MODE_640")).await?;
        checks.push("Rename keeps apostrophe/Unicode; POSIX mode 0640 is reported by SFTP and stat on the server".into());
        files.execute(FileAction::Delete{path:renamed},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        checks.push("SFTP deletes the renamed file".into());
        let edit_path=format!("{remote}/editor.txt");
        files.execute(FileAction::Upload{path:edit_path.clone(),local:unicode.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let staged=assets.join("engine-editor.txt");
        let baseline=files.execute(FileAction::Download{path:edit_path.clone(),local:staged.clone(),edit:true},cancel.clone()).await.map_err(anyhow::Error::msg)?.fingerprint.context("No editor baseline")?;
        tokio::fs::write(&staged,b"edited\n").await?;
        let next=files.execute(FileAction::Replace{path:edit_path.clone(),local:staged.clone(),expected:baseline.clone(),expected_local:None},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(next.fingerprint.is_some(),"No verified upload fingerprint");
        let conflict=files.execute(FileAction::Replace{path:edit_path,local:staged,expected:baseline,expected_local:None},cancel.clone()).await;
        ensure!(conflict.is_err_and(|e|e.contains("Conflict")),"Stale baseline did not block replacement");
        checks.push("Verified external-edit replacement succeeds and stale baseline conflicts block".into());
        Ok(())
    }.await;
    live.disconnect();
    std::fs::write(
        folder.join("engine-report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passed":result.is_ok(),"checks":checks,"error":result.as_ref().err().map(|e|format!("{e:#}"))}),
        )?,
    )?;
    result
}
