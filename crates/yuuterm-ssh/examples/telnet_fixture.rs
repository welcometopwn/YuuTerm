//! Loopback-only TELNET fixture for the native capture build; no OS login/shell.
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use yuuterm_core::{Profile, Protocol};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Supply a capture output directory"))?,
    );
    std::fs::create_dir_all(&dir)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let profile = Profile {
        id: 1,
        name: "TELNET fixture".into(),
        protocol: Protocol::Telnet,
        host: "127.0.0.1".into(),
        port: listener.local_addr()?.port(),
        username: "fixture".into(),
        ..Default::default()
    };
    std::fs::write(
        dir.join("profile.json"),
        serde_json::to_vec_pretty(&profile)?,
    )?;
    println!("Loopback TELNET capture fixture ready");
    let search_flood = std::env::args().nth(2).as_deref() == Some("--search-flood");
    tokio::time::timeout(Duration::from_secs(180),async {
        loop {
            let (mut stream,_)=listener.accept().await?;
            tokio::spawn(async move {
                let result=async {
                    stream.set_nodelay(true)?;
                    stream.write_all(&[255,251,1,255,251,0,255,253,0,255,253,24,255,253,31,255,250,24,1,255,240]).await?;
                    stream.write_all(b"\x1b[32mReady\x1b[0m\r\nLocal TELNET protocol fixture\r\nwarning unencrypted 192.0.2.42\r\nNo OS account, shell or credentials\r\nfixture> ").await?;
                    let mut state=0u8;let mut input=[0u8;16384];
                    let mut command=Vec::with_capacity(64);
                    loop {
                        let count=stream.read(&mut input).await?;if count==0{break;}
                        let mut echo=Vec::new();
                        for &byte in &input[..count] {
                            state=match state {
                                0=>if byte==255{1}else{echo.push(byte);0},
                                1=>match byte {255=>{echo.extend([255,255]);0},251..=254=>2,250=>3,_=>0},
                                2=>0, // option value
                                3=>if byte==255{4}else{3},
                                4=>if byte==240{0}else{3},
                                _=>0,
                            };
                        }
                        if search_flood {
                            for byte in echo {
                                // Initial input can precede BINARY negotiation:
                                // NVT encodes CR as CR NUL. Fixture commands are ASCII.
                                if byte == 0 { continue; }
                                if byte == b'\r' || byte == b'\n' {
                                    if command == b"YT_SEARCH_PREP" {
                                        stream.write_all(b"\x1b[3J\x1b[2J\x1b[Htrim-me-before-flood\r\n").await?;
                                    } else if command == b"YT_SEARCH_FLOOD_500" || command == b"YT_SEARCH_FLOOD_10000" {
                                        let limit=if command.ends_with(b"10000"){10000}else{500};
                                        for batch in 0..200 {
                                            let mut output=String::with_capacity(4096);
                                            for row in batch*100..batch*100+100 {
                                                use std::fmt::Write;
                                                writeln!(output,"row-{row:05} flood-target\r")?;
                                            }
                                            stream.write_all(output.as_bytes()).await?;
                                            tokio::time::sleep(Duration::from_millis(30)).await;
                                        }
                                        stream.write_all(format!("flood-done-{limit}\r\n").as_bytes()).await?;
                                    } else if !command.is_empty() {
                                        stream.write_all(&command).await?;
                                        stream.write_all(b"\r\n").await?;
                                    }
                                    command.clear();
                                } else {
                                    anyhow::ensure!(command.len()<64,"fixture command bound");
                                    command.push(byte);
                                }
                            }
                        } else if !echo.is_empty(){stream.write_all(&echo).await?;}
                    }
                    Ok::<_,anyhow::Error>(())
                }.await;
                if result.is_err(){ /* fixture disconnects are intentionally quiet */ }
            });
        }
        #[allow(unreachable_code)] Ok::<_,anyhow::Error>(())
    }).await.ok();
    Ok(())
}
