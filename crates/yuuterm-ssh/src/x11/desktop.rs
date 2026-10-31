//! X server readiness: the bundled server (`managed`) when this installation ships
//! one, otherwise read-only discovery of an already running, authenticated local X server.
//! Inventory and authority reads run on a blocking worker; socket probes are async.
use super::LocalX11;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Read, net::SocketAddr, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

const LIMIT: usize = 64 * 1024;
const AUTH: &[u8] = b"MIT-MAGIC-COOKIE-1";
#[derive(Serialize, Debug)]
pub struct Probe {
    pub available: bool,
    pub message: String,
}
#[derive(Deserialize)]
struct Inventory {
    installed: Vec<String>,
    candidates: Vec<Candidate>,
    hostname: String,
}
#[derive(Deserialize)]
struct Candidate {
    server: String,
    port: u16,
    ipv6: bool,
    authority: PathBuf,
    display: u16,
}
pub async fn probe() -> Probe {
    if let Some(server) = super::managed::available() {
        return Probe {
            available: true,
            message: format!(
                "Bundled {server}: starts when an X11 session connects and stops after the last one. It accepts only YuuTerm's per-start cookie on 127.0.0.1, with clipboard off. Remote X clients share its desktop access."
            ),
        };
    }
    match discover().await {
        Ok((_, server)) => Probe {
            available: true,
            message: format!(
                "{server}: authenticated loopback-only X server ready. Remote X clients share its desktop access."
            ),
        },
        Err(e) => Probe {
            available: false,
            message: format!(
                "X11 unavailable: {e:#}. This installation has no bundled X server (x11 folder beside the executable). Otherwise a running VcXsrv with cookie authentication, -noclipboard and loopback-only listeners is required; set DISPLAY and XAUTHORITY. X410 detection is supported, but its authentication/clipboard policy is not."
            ),
        },
    }
}
pub async fn local() -> Result<LocalX11> {
    if super::managed::available().is_some() {
        return super::managed::local().await;
    }
    Ok(discover().await?.0)
}

/// A correct cookie alone cannot detect -ac / host-based access. Require a second
/// connection with a random incorrect cookie to be refused.
pub(super) async fn verify(server: &LocalX11) -> Result<()> {
    ensure!(
        setup(server, &server.cookie).await?,
        "X server rejected authority"
    );
    let mut wrong = Zeroizing::new(rand::random::<[u8; 16]>());
    if *wrong == *server.cookie {
        wrong[0] ^= 1;
    }
    ensure!(
        !setup(server, &wrong).await?,
        "X server accepts unauthenticated clients"
    );
    Ok(())
}

async fn discover() -> Result<(LocalX11, String)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let inventory = tokio::task::spawn_blocking(inventory).await??;
        select(inventory).await
    })
    .await
    .context("X server discovery timed out")?
}

async fn select(inventory: Inventory) -> Result<(LocalX11, String)> {
    ensure!(
        !inventory.installed.is_empty(),
        "no installed VcXsrv or X410 detected"
    );
    ensure!(
        inventory.candidates.len() <= 16,
        "too many X server candidates"
    );
    for candidate in inventory.candidates {
        let hostname = inventory.hostname.clone();
        let display = candidate.display;
        let path = candidate.authority;
        let cookie = tokio::task::spawn_blocking(move || {
            local_regular_file(&path)?;
            let mut bytes = Zeroizing::new(Vec::new());
            std::fs::File::open(path)?
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= LIMIT, "authority file exceeds limit");
            authority(&bytes, display, hostname.as_bytes())
        })
        .await?;
        let Ok(cookie) = cookie else { continue };
        let endpoint = SocketAddr::new(
            if candidate.ipv6 {
                std::net::Ipv6Addr::LOCALHOST.into()
            } else {
                std::net::Ipv4Addr::LOCALHOST.into()
            },
            candidate.port,
        );
        let local = LocalX11::new(endpoint, *cookie)?;
        if matches!(
            tokio::time::timeout(Duration::from_secs(3), verify(&local)).await,
            Ok(Ok(()))
        ) {
            return Ok((local, candidate.server));
        }
    }
    bail!("no running cookie-authenticated, loopback-only X server matched the selected display")
}

fn local_regular_file(path: &std::path::Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::fs::MetadataExt,
            path::{Component, Prefix},
        };
        let mut parts = path.components();
        ensure!(
            matches!(parts.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))),
            "Xauthority must use an absolute local drive path"
        );
        ensure!(
            matches!(parts.next(), Some(Component::RootDir)),
            "Xauthority must be absolute"
        );
        let mut current = path.ancestors().collect::<Vec<_>>();
        current.reverse();
        ensure!(current.len() <= 64, "Xauthority path exceeds limit");
        for component in parts {
            ensure!(
                matches!(component, Component::Normal(_)),
                "Xauthority path must not traverse parent directories"
            );
        }
        for item in current {
            let metadata = std::fs::symlink_metadata(item)?;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "Xauthority path must not traverse reparse points"
            );
        }
    }
    let metadata = std::fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= LIMIT as u64,
        "Xauthority must be a bounded regular file"
    );
    Ok(())
}

fn authority(bytes: &[u8], display: u16, hostname: &[u8]) -> Result<Zeroizing<[u8; 16]>> {
    fn field<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
        ensure!(input.len() >= 2, "truncated Xauthority length");
        let len = u16::from_be_bytes([input[0], input[1]]) as usize;
        *input = &input[2..];
        ensure!(input.len() >= len, "truncated Xauthority field");
        let (value, rest) = input.split_at(len);
        *input = rest;
        Ok(value)
    }
    let mut input = bytes;
    let mut found = None;
    let number = display.to_string();
    while !input.is_empty() {
        ensure!(input.len() >= 2, "truncated Xauthority family");
        let family = u16::from_be_bytes([input[0], input[1]]);
        input = &input[2..];
        let address = field(&mut input)?;
        let selected = field(&mut input)?;
        let name = field(&mut input)?;
        let cookie = field(&mut input)?;
        let local = match family {
            0 => address == [127, 0, 0, 1],
            6 => address == std::net::Ipv6Addr::LOCALHOST.octets(),
            256 => !hostname.is_empty() && address.eq_ignore_ascii_case(hostname),
            _ => false, // Includes FamilyWild; never guess a remote authority.
        };
        if local && selected == number.as_bytes() && name == AUTH {
            ensure!(
                cookie.len() == 16 && found.is_none(),
                "invalid or ambiguous authority"
            );
            found = Some(Zeroizing::new(cookie.try_into().unwrap()));
        }
    }
    found.context("no exact local MIT-MAGIC-COOKIE-1 authority")
}

async fn setup(server: &LocalX11, cookie: &[u8; 16]) -> Result<bool> {
    let mut socket = server.connect().await?;
    let mut request = Zeroizing::new(vec![0u8; 48]);
    request[0] = b'l';
    request[2..4].copy_from_slice(&11u16.to_le_bytes());
    request[6..8].copy_from_slice(&18u16.to_le_bytes());
    request[8..10].copy_from_slice(&16u16.to_le_bytes());
    request[12..30].copy_from_slice(AUTH);
    request[32..].copy_from_slice(cookie);
    socket.write_all(&request).await?;
    let mut header = [0; 8];
    socket.read_exact(&mut header).await?;
    ensure!(header[0] <= 1, "unsupported X authentication exchange");
    let len = u16::from_le_bytes([header[6], header[7]]) as usize * 4;
    ensure!(len <= LIMIT, "oversized X setup response");
    if header[0] == 1 {
        ensure!(
            u16::from_le_bytes([header[2], header[3]]) == 11 && len >= 32,
            "invalid X setup success"
        );
    }
    let mut body = Zeroizing::new(vec![0; len]);
    socket.read_exact(&mut body).await?;
    Ok(header[0] == 1)
}

#[cfg(windows)]
fn inventory() -> Result<Inventory> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::Instant,
    };
    let mut child = Command::new("powershell.exe")
        .creation_flags(0x08000000)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            include_str!("../../../../scripts/Inspect-X11.ps1"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .context("X discovery pipe unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "X server inventory failed");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("X server inventory timed out")
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("X inventory worker failed"))??;
    ensure!(bytes.len() <= LIMIT, "X inventory exceeds limit");
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(not(windows))]
fn inventory() -> Result<Inventory> {
    bail!("installed X server discovery is available on Windows only")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(family: u16, address: &[u8], number: &[u8], cookie: &[u8]) -> Vec<u8> {
        let mut bytes = family.to_be_bytes().to_vec();
        for field in [address, number, AUTH, cookie] {
            bytes.extend((field.len() as u16).to_be_bytes());
            bytes.extend(field);
        }
        bytes
    }
    #[test]
    fn authority_is_exact_bounded_and_never_wildcard() {
        let bytes = entry(256, b"fixture", b"0", &[1; 16]);
        assert_eq!(*authority(&bytes, 0, b"FIXTURE").unwrap(), [1; 16]);
        for end in 0..bytes.len() {
            assert!(authority(&bytes[..end], 0, b"fixture").is_err());
        }
        for invalid in [
            entry(65535, b"", b"0", &[1; 16]),
            entry(0, &[192, 0, 2, 1], b"0", &[1; 16]),
            entry(0, &[127, 0, 0, 1], b"1", &[1; 16]),
            entry(256, b"fixture", b"0", &[1; 17]),
            [bytes.clone(), bytes].concat(),
        ] {
            assert!(authority(&invalid, 0, b"fixture").is_err());
        }
    }
    #[tokio::test]
    async fn setup_checks_success_refusal_truncation_and_deadline() {
        for (status, body_len, truncated, expected) in [
            (1, 8, false, Some(true)),
            (0, 0, false, Some(false)),
            (2, 0, false, None),
            (1, u16::MAX, false, None),
            (1, 1, true, None),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut request = [0; 48];
                s.read_exact(&mut request).await.unwrap();
                assert_eq!(&request[32..], &[7; 16]);
                let mut h = [0; 8];
                h[0] = status;
                h[2..4].copy_from_slice(&11u16.to_le_bytes());
                h[6..].copy_from_slice(&body_len.to_le_bytes());
                s.write_all(&h).await.unwrap();
                if !truncated {
                    if body_len <= 8 {
                        s.write_all(&vec![0; body_len as usize * 4]).await.unwrap();
                    }
                    s.shutdown().await.unwrap();
                }
            });
            let result = setup(&LocalX11::new(endpoint, [7; 16]).unwrap(), &[7; 16]).await;
            assert_eq!(result.ok(), expected);
            peer.await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (_s, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                setup(&LocalX11::new(endpoint, [7; 16]).unwrap(), &[7; 16])
            )
            .await
            .is_err()
        );
        peer.abort();
    }

    #[tokio::test]
    async fn discovery_requires_a_cookie_and_rejection_of_incorrect_authentication() {
        assert!(
            select(Inventory {
                installed: vec![],
                candidates: vec![],
                hostname: String::new()
            })
            .await
            .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("authority");
        std::fs::write(&path, entry(256, b"fixture", b"0", &[7; 16])).unwrap();
        for unauthenticated in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let peer = tokio::spawn(async move {
                for attempt in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = [0; 48];
                    socket.read_exact(&mut request).await.unwrap();
                    assert_eq!(request[32..] == [7; 16], attempt == 0);
                    let accepted = attempt == 0 || unauthenticated;
                    let mut header = [0; 8];
                    header[0] = u8::from(accepted);
                    header[2] = 11;
                    if accepted {
                        header[6] = 8;
                    }
                    socket.write_all(&header).await.unwrap();
                    if accepted {
                        socket.write_all(&[0; 32]).await.unwrap();
                    }
                }
            });
            let selected = select(Inventory {
                installed: vec!["VcXsrv fixture".into()],
                hostname: "fixture".into(),
                candidates: vec![Candidate {
                    server: "VcXsrv fixture".into(),
                    port,
                    ipv6: false,
                    authority: path.clone(),
                    display: 0,
                }],
            })
            .await;
            assert_eq!(selected.is_ok(), !unauthenticated);
            peer.await.unwrap();
        }
        std::fs::write(&path, vec![0; LIMIT + 1]).unwrap();
        assert!(
            select(Inventory {
                installed: vec!["fixture".into()],
                hostname: "fixture".into(),
                candidates: vec![Candidate {
                    server: "fixture".into(),
                    port: 6000,
                    ipv6: false,
                    authority: path,
                    display: 0
                }]
            })
            .await
            .is_err()
        );
    }
}
