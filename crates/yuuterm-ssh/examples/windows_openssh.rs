//! Explicitly invoked external test-server qualification; never a default test.
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
    .context("Windows OpenSSH deadline")?
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
    let prefix = format!("/C:/Users/{}/yuuterm-test/run-", profile.username);
    ensure!(
        remote.starts_with(&prefix)
            && remote[prefix.len()..].len() == 32
            && remote[prefix.len()..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Remote writes must stay in the generated test subtree"
    );
    let engine = Engine::new(folder.join("host-keys.json"))?;
    let live = engine.connect(profile, Arc::new(|| {}))?;
    wait(&live, |s| s.connected).await?;
    let mut checks = vec!["Key-file SSH authentication with the isolated host pin".to_owned()];
    let result:Result<()>=async {
        live.send(Command::Resize{rows:30,cols:100}).map_err(anyhow::Error::msg)?;
        live.send(Command::Input(b"if(Get-Module PSReadLine){Set-PSReadLineOption -HistorySaveStyle SaveNothing}; Write-Output ('YT_'+'SHELL_OK'); Write-Output ([char]0x03a9); Write-Output ('YT_SIZE_'+$Host.UI.RawUI.WindowSize.Width+'x'+$Host.UI.RawUI.WindowSize.Height)\r".to_vec())).map_err(anyhow::Error::msg)?;
        wait(&live, |_|text(&live).contains("YT_SHELL_OK") && text(&live).contains('\u{03a9}')).await?;
        checks.push("Actual PowerShell PTY output and Unicode Omega rendered by the engine terminal".into());
        wait(&live, |_|text(&live).contains("YT_SIZE_100x30")).await?;
        checks.push("PTY resize independently observed as 100x30 by PowerShell RawUI".into());
        let files=live.files();
        let cancel=CancellationToken::new();
        for path in [remote.clone(),remote.trim_start_matches('/').to_owned(),remote.trim_start_matches('/').replace('/',"\\")] {
            let done=files.execute(FileAction::Resolve{path},cancel.clone()).await.map_err(anyhow::Error::msg)?;
            ensure!(done.resolved.context("No resolved path")?.directory,"Test run must be a directory");
        }
        checks.push("SFTP resolves slash-prefixed drive, drive-slash and drive-backslash forms".into());
        live.send(Command::Browse(remote.clone())).map_err(anyhow::Error::msg)?;
        wait(&live,|s|s.path==remote).await?;
        checks.push("SFTP browse into the generated Windows directory".into());
        let path=format!("{remote}/roundtrip.bin");
        let original=(0..65539).map(|i|(i%251)as u8).collect::<Vec<_>>();
        let local=assets.join("roundtrip.bin");tokio::fs::write(&local,&original).await?;
        files.execute(FileAction::Upload{path:path.clone(),local:local.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let download=assets.join("download.bin");
        files.execute(FileAction::Download{path:path.clone(),local:download.clone(),edit:false},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(tokio::fs::read(download).await?==original,"Binary roundtrip mismatch");
        checks.push("SFTP upload/download preserve 65,539 binary bytes exactly".into());
        let props=files.execute(FileAction::Properties{path:path.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?.properties.context("No properties")?;
        std::fs::write(folder.join("mapped-properties.json"),serde_json::to_vec_pretty(&props)?)?;
        let directory=format!("{remote}/tree");
        files.execute(FileAction::Mkdir{path:directory.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        files.execute(FileAction::Create{path:format!("{directory}/empty.txt")},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let unicode=assets.join("unicode.txt");let content="CRLF\r\nUnicode \u{03a9} \u{65e5}\r\n";tokio::fs::write(&unicode,content).await?;
        files.execute(FileAction::Upload{path:format!("{directory}/space \u{03a9}.txt"),local:unicode.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let tree=assets.join("tree-download");
        files.execute(FileAction::DownloadTree{path:directory.clone(),local:tree.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        ensure!(tokio::fs::read(tree.join("space \u{03a9}.txt")).await?==content.as_bytes(),"Directory Unicode/CRLF mismatch");
        ensure!(tokio::fs::metadata(tree.join("empty.txt")).await?.len()==0,"New file is not empty");
        checks.push("New file/folder and recursive directory download preserve Unicode names and CRLF bytes".into());
        files.execute(FileAction::Rename{path:path.clone(),name:"renamed.bin".into()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let renamed=format!("{remote}/renamed.bin");
        files.execute(FileAction::Permissions{path:renamed.clone(),mode:0o444,recursive:false},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let read_only=files.execute(FileAction::Properties{path:renamed.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?.properties.context("No mapped permissions")?;
        std::fs::write(folder.join("readonly-properties.json"),serde_json::to_vec_pretty(&read_only)?)?;
        files.execute(FileAction::Permissions{path:renamed.clone(),mode:0o644,recursive:false},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        checks.push("SFTP rename and permission requests complete; mapped attributes recorded separately".into());
        files.execute(FileAction::Delete{path:renamed},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        checks.push("SFTP deletes the renamed file".into());
        let edit_path=format!("{remote}/editor.txt");
        files.execute(FileAction::Upload{path:edit_path.clone(),local:unicode.clone()},cancel.clone()).await.map_err(anyhow::Error::msg)?;
        let staged=assets.join("engine-editor.txt");
        let baseline=files.execute(FileAction::Download{path:edit_path.clone(),local:staged.clone(),edit:true},cancel.clone()).await.map_err(anyhow::Error::msg)?.fingerprint.context("No editor baseline")?;
        tokio::fs::write(&staged,b"edited\r\n").await?;
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
