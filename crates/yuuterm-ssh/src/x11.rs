//! Opt-in X11 transport to an authenticated loopback X server: the bundled
//! Cygwin/X server (`managed`) or an installed one (`desktop`).
use anyhow::{Context, Result, ensure};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub mod desktop;
pub mod managed;

const AUTH: &[u8] = b"MIT-MAGIC-COOKIE-1";
const MAX_CONNECTIONS: usize = 8;

/// Runtime-only local-server credentials. Never serialized or stored in profiles.
pub struct LocalX11 {
    endpoint: SocketAddr,
    cookie: Zeroizing<[u8; 16]>,
    /// Secret of Cygwin's AF_LOCAL emulation when the server is the bundled XWin.
    af_local: Option<Zeroizing<[u8; 16]>>,
    /// Keeps the bundled server running while a session uses it.
    _server: Option<Arc<dyn std::any::Any + Send + Sync>>,
}
impl std::fmt::Debug for LocalX11 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalX11")
            .field("endpoint", &self.endpoint)
            .field("cookie", &"<redacted>")
            .finish()
    }
}
impl LocalX11 {
    pub fn new(endpoint: SocketAddr, cookie: [u8; 16]) -> Result<Self> {
        ensure!(
            endpoint.ip().is_loopback() && endpoint.port() != 0,
            "X11 requires a concrete loopback-only endpoint"
        );
        Ok(Self {
            endpoint,
            cookie: Zeroizing::new(cookie),
            af_local: None,
            _server: None,
        })
    }
    pub(crate) fn cygwin(
        port: u16,
        secret: [u8; 16],
        cookie: [u8; 16],
        server: Arc<dyn std::any::Any + Send + Sync>,
    ) -> Self {
        Self {
            endpoint: (std::net::Ipv4Addr::LOCALHOST, port).into(),
            cookie: Zeroizing::new(cookie),
            af_local: Some(Zeroizing::new(secret)),
            _server: Some(server),
        }
    }
    pub(crate) async fn connect(&self) -> Result<TcpStream> {
        let mut stream = TcpStream::connect(self.endpoint).await?;
        stream.set_nodelay(true)?;
        if let Some(secret) = &self.af_local {
            af_local_handshake(&mut stream, secret).await?;
        }
        Ok(stream)
    }
}

/// Client side of Cygwin's emulated AF_LOCAL stream: both peers prove the secret
/// from the socket file, then exchange (pid, uid, gid). Unknown ids (-1) grant no
/// SI:localuser access, so the X cookie stays the only authorization.
async fn af_local_handshake<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
    secret: &[u8; 16],
) -> Result<()> {
    stream.write_all(secret).await?;
    let mut echo = Zeroizing::new([0u8; 16]);
    stream.read_exact(&mut *echo).await?;
    ensure!(
        bool::from(echo.ct_eq(secret)),
        "Local X server failed the Cygwin socket handshake"
    );
    let mut credentials = [0xff; 12];
    credentials[..4].copy_from_slice(&std::process::id().to_le_bytes());
    stream.write_all(&credentials).await?;
    stream.read_exact(&mut credentials).await?;
    Ok(())
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub active: usize,
    pub authenticated: u64,
    pub rejected: u64,
}
pub(crate) struct Forward {
    local: LocalX11,
    fake: Zeroizing<[u8; 16]>,
    pub permits: Arc<Semaphore>,
    authenticated: AtomicU64,
    rejected: AtomicU64,
}
impl Forward {
    pub fn new(local: LocalX11) -> Self {
        Self {
            local,
            fake: Zeroizing::new(rand::random()),
            permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            authenticated: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }
    }
    pub fn cookie(&self) -> String {
        self.fake.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    pub fn stats(&self) -> Stats {
        Stats {
            active: MAX_CONNECTIONS - self.permits.available_permits(),
            authenticated: self.authenticated.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }
    pub fn reject(&self) {
        self.rejected.fetch_add(1, Ordering::Relaxed);
    }
    pub async fn bridge<T>(&self, mut channel: T, cancel: CancellationToken) -> Result<()>
    where
        T: AsyncRead + AsyncWrite + Unpin,
    {
        tokio::select! {
            biased;
            _=cancel.cancelled()=>Ok(()),
            result=async {
                let setup=tokio::time::timeout(Duration::from_secs(10),read_setup(&mut channel,&self.fake,&self.local.cookie)).await.context("X11 setup timed out")??;
                // Invalid cookies never cause even a local TCP connection.
                let mut local=tokio::time::timeout(Duration::from_secs(5),self.local.connect()).await.context("Local X server timed out")??;
                tokio::time::timeout(Duration::from_secs(5),local.write_all(&setup)).await.context("Local X server setup write timed out")??;
                self.authenticated.fetch_add(1,Ordering::Relaxed);
                tokio::io::copy_bidirectional_with_sizes(&mut channel,&mut local,16384,16384).await?;
                Ok(())
            }=>result,
        }
    }
}

async fn read_setup<T: AsyncRead + Unpin>(
    stream: &mut T,
    fake: &[u8; 16],
    real: &[u8; 16],
) -> Result<Zeroizing<Vec<u8>>> {
    let mut header = [0u8; 12];
    stream.read_exact(&mut header).await?;
    let little = match header[0] {
        b'l' => true,
        b'B' => false,
        _ => anyhow::bail!("Invalid X11 byte order"),
    };
    let number = |offset| {
        let bytes = [header[offset], header[offset + 1]];
        if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    ensure!(
        number(2) == 11 && number(4) == 0,
        "Unsupported X11 protocol version"
    );
    // Exact lengths avoid attacker-sized allocations and reject other auth methods.
    ensure!(
        number(6) == AUTH.len() as u16 && number(8) == 16,
        "Unsupported X11 authentication setup"
    );
    let name_padded = (AUTH.len() + 3) & !3;
    let mut setup = Zeroizing::new(vec![0; 12 + name_padded + 16]);
    setup[..12].copy_from_slice(&header);
    stream.read_exact(&mut setup[12..]).await?;
    ensure!(
        &setup[12..12 + AUTH.len()] == AUTH,
        "Unsupported X11 authentication protocol"
    );
    let start = 12 + name_padded;
    ensure!(
        bool::from(setup[start..start + 16].ct_eq(fake)),
        "X11 authentication failed"
    );
    setup[start..start + 16].copy_from_slice(real);
    Ok(setup)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(little: bool, cookie: [u8; 16]) -> Vec<u8> {
        let mut data = vec![0; 48];
        data[0] = if little { b'l' } else { b'B' };
        for (offset, value) in [(2, 11u16), (6, 18), (8, 16)] {
            data[offset..offset + 2].copy_from_slice(&if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            });
        }
        data[12..30].copy_from_slice(AUTH);
        data[32..48].copy_from_slice(&cookie);
        data
    }
    #[tokio::test]
    async fn setup_auth_is_bounded_and_rewrites_both_byte_orders() {
        for little in [true, false] {
            let data = setup(little, [1; 16]);
            let actual = read_setup(&mut &data[..], &[1; 16], &[2; 16])
                .await
                .unwrap();
            assert!(actual[32..48].iter().all(|b| *b == 2));
            assert_eq!(&actual[..32], &data[..32]);
        }
        for mut data in [
            setup(true, [3; 16]),
            setup(true, [1; 16]),
            setup(true, [1; 16]),
            setup(true, [1; 16]),
        ]
        .into_iter()
        .enumerate()
        {
            match data.0 {
                1 => data.1[0] = b'?',
                2 => data.1[6..8].copy_from_slice(&65535u16.to_le_bytes()),
                3 => data.1.truncate(20),
                _ => {}
            }
            assert!(
                read_setup(&mut &data.1[..], &[1; 16], &[2; 16])
                    .await
                    .is_err()
            );
        }
        assert!(LocalX11::new("0.0.0.0:6000".parse().unwrap(), [0; 16]).is_err());
        assert!(LocalX11::new("192.0.2.1:6000".parse().unwrap(), [0; 16]).is_err());
        let config = LocalX11::new("127.0.0.1:6000".parse().unwrap(), [0xab; 16]).unwrap();
        assert!(format!("{config:?}").contains("<redacted>"));
    }
    #[tokio::test]
    async fn af_local_handshake_proves_the_secret_both_ways() {
        for honest in [true, false] {
            let (mut client, mut server) = tokio::io::duplex(64);
            let peer = tokio::spawn(async move {
                let mut secret = [0; 16];
                server.read_exact(&mut secret).await.unwrap();
                assert_eq!(secret, [3; 16]);
                server
                    .write_all(&if honest { secret } else { [4; 16] })
                    .await
                    .unwrap();
                if honest {
                    let mut credentials = [0; 12];
                    server.read_exact(&mut credentials).await.unwrap();
                    assert_eq!(&credentials[4..], &[0xff; 8]);
                    server.write_all(&[0; 12]).await.unwrap();
                }
            });
            assert_eq!(
                af_local_handshake(&mut client, &[3; 16]).await.is_ok(),
                honest
            );
            peer.await.unwrap();
        }
    }
    #[tokio::test(start_paused = true)]
    async fn stalled_setup_has_a_deadline_and_cancel_is_immediate() {
        let forward = Forward::new(LocalX11::new("127.0.0.1:1".parse().unwrap(), [5; 16]).unwrap());
        let (_peer, stream) = tokio::io::duplex(64);
        let start = tokio::time::Instant::now();
        let error = forward
            .bridge(stream, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("setup timed out"));
        assert_eq!(start.elapsed(), Duration::from_secs(10));
        let (_peer, stream) = tokio::io::duplex(64);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let start = tokio::time::Instant::now();
        forward.bridge(stream, cancel).await.unwrap();
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
