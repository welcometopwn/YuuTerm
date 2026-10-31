//! Experimental RFB security negotiation only, not a connected VNC session.
//! No credentials are accepted here. The caller must validate the server and
//! complete TLS before authentication or displaying remote content. The `vnc`
//! worker implements that upgrade; this module remains plaintext preflight only.
use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const BANNER: &[u8; 12] = b"RFB 003.008\n";
const VENCRYPT: u8 = 19;
const X509_PLAIN: u32 = 262;

/// Authentication selection is explicit; never downgrade to another mode.
#[derive(Clone, Copy)]
pub enum TlsAuth {
    Plain,
    Vnc,
}
impl TlsAuth {
    fn subtype(self) -> u32 {
        match self {
            Self::Plain => X509_PLAIN,
            Self::Vnc => 261,
        }
    }
}

/// The stream is still plaintext at the exact TLS-upgrade boundary.
/// This value does not establish certificate trust or authenticate either peer.
pub struct AwaitingTls<S> {
    stream: S,
}
impl<S> AwaitingTls<S> {
    pub fn into_plaintext_stream_for_tls_upgrade(self) -> S {
        self.stream
    }
}

/// Conservative default target: RFB 3.8, VeNCrypt 0.2, X509Plain.
/// Refuse None, unencrypted VNC auth, anonymous TLS and unimplemented subtypes.
/// The entire preflight has one deadline; trickling bytes cannot extend it.
pub async fn negotiate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    cancel: CancellationToken,
) -> Result<AwaitingTls<S>> {
    negotiate_auth(stream, cancel, TlsAuth::Plain).await
}

pub async fn negotiate_auth<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    cancel: CancellationToken,
    auth: TlsAuth,
) -> Result<AwaitingTls<S>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => bail!("VNC preflight cancelled"),
        result = tokio::time::timeout(std::time::Duration::from_secs(10), handshake(stream, auth)) =>
            result.context("VNC preflight deadline exceeded")?,
    }
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    auth: TlsAuth,
) -> Result<AwaitingTls<S>> {
    let mut banner = [0; 12];
    stream
        .read_exact(&mut banner)
        .await
        .context("Reading RFB version")?;
    if &banner != BANNER {
        bail!("VNC prototype requires RFB 3.8")
    }
    stream.write_all(BANNER).await?;
    stream.flush().await?;
    let count = stream.read_u8().await? as usize;
    if count == 0 {
        bail!("VNC server refused security negotiation")
    }
    let mut security = vec![0; count]; // Wire count is u8: allocation <= 255 bytes.
    stream.read_exact(&mut security).await?;
    if !security.contains(&VENCRYPT) {
        bail!("VNC requires VeNCrypt; insecure fallback is disabled")
    }
    stream.write_all(&[VENCRYPT]).await?;
    stream.flush().await?;
    let mut version = [0; 2];
    stream.read_exact(&mut version).await?;
    if version != [0, 2] {
        bail!("VNC prototype requires VeNCrypt 0.2")
    }
    stream.write_all(&[0, 2]).await?;
    stream.flush().await?;
    if stream.read_u8().await? != 0 {
        bail!("VNC server rejected VeNCrypt version")
    }
    let count = stream.read_u8().await?;
    if count == 0 {
        bail!("VNC server offered no authentication subtypes")
    }
    let mut supported = false;
    for _ in 0..count {
        supported |= stream.read_u32().await? == auth.subtype();
    }
    if !supported {
        bail!("VNC requires the selected X509 authentication mode; fallback is disabled")
    }
    stream.write_u32(auth.subtype()).await?;
    stream.flush().await?;
    if stream.read_u8().await? != 1 {
        bail!("VNC server refused the TLS upgrade")
    }
    Ok(AwaitingTls { stream })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    async fn offer<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, subtypes: &[u32]) {
        // Fragment the server banner deliberately; read_exact must reassemble it.
        for byte in BANNER {
            s.write_all(&[*byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        let mut banner = [0; 12];
        s.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, BANNER);
        s.write_all(&[3, 1, 2, VENCRYPT]).await.unwrap();
        assert_eq!(s.read_u8().await.unwrap(), VENCRYPT);
        s.write_all(&[0, 2]).await.unwrap();
        let mut version = [0; 2];
        s.read_exact(&mut version).await.unwrap();
        assert_eq!(version, [0, 2]);
        s.write_all(&[0, subtypes.len() as u8]).await.unwrap();
        for subtype in subtypes {
            s.write_u32(*subtype).await.unwrap();
        }
    }

    #[tokio::test]
    async fn real_loopback_stops_at_tls_boundary_without_credentials_or_client_init() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            offer(&mut s, &[256, 257, 258, 259, 260, 261, 262]).await;
            assert_eq!(s.read_u32().await.unwrap(), X509_PLAIN);
            s.write_u8(1).await.unwrap();
            let mut extra = [0];
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(80), s.read(&mut extra))
                    .await
                    .is_err(),
                "Client sent data before TLS"
            );
        });
        let stream = TcpStream::connect(address).await.unwrap();
        let pending = negotiate(stream, CancellationToken::new()).await.unwrap();
        peer.await.unwrap();
        drop(pending.into_plaintext_stream_for_tls_upgrade());
    }

    #[tokio::test]
    async fn unsafe_security_types_never_receive_a_selection() {
        for list in [&[1][..], &[2][..], &[1, 2][..]] {
            let (client, mut peer) = tokio::io::duplex(512);
            let task = tokio::spawn(negotiate(client, CancellationToken::new()));
            peer.write_all(BANNER).await.unwrap();
            let mut banner = [0; 12];
            peer.read_exact(&mut banner).await.unwrap();
            peer.write_u8(list.len() as u8).await.unwrap();
            peer.write_all(list).await.unwrap();
            assert!(task.await.unwrap().is_err());
            assert_eq!(
                peer.read_u8().await.unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
        }
    }

    #[tokio::test]
    async fn unsafe_subtypes_and_upgrade_refusal_fail_closed() {
        for subtypes in [vec![256, 257, 258, 259, 260, 261], vec![262]] {
            let (client, mut peer) = tokio::io::duplex(4096);
            let task = tokio::spawn(negotiate(client, CancellationToken::new()));
            offer(&mut peer, &subtypes).await;
            if subtypes == [262] {
                assert_eq!(peer.read_u32().await.unwrap(), 262);
                peer.write_u8(0).await.unwrap();
            }
            assert!(task.await.unwrap().is_err());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn one_deadline_bounds_stalled_peers_and_precancelled_connections_write_nothing() {
        let (client, _peer) = tokio::io::duplex(512);
        assert!(
            negotiate(client, CancellationToken::new())
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("deadline")
        );
        let (client, mut peer) = tokio::io::duplex(512);
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(negotiate(client, cancel).await.is_err());
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn every_truncated_server_transcript_is_refused() {
        let mut wire = BANNER.to_vec();
        wire.extend_from_slice(&[3, 1, 2, 19, 0, 2, 0, 1]);
        wire.extend_from_slice(&262u32.to_be_bytes());
        wire.push(1);
        for end in 0..wire.len() {
            let (client, mut peer) = tokio::io::duplex(512);
            peer.write_all(&wire[..end]).await.unwrap();
            peer.shutdown().await.unwrap();
            assert!(
                negotiate(client, CancellationToken::new()).await.is_err(),
                "Accepted truncated prefix {end}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn trickling_bytes_cannot_reset_the_overall_deadline() {
        let (client, mut peer) = tokio::io::duplex(512);
        let writer = tokio::spawn(async move {
            for byte in BANNER {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if peer.write_u8(*byte).await.is_err() {
                    break;
                }
            }
        });
        let at = tokio::time::Instant::now();
        assert!(negotiate(client, CancellationToken::new()).await.is_err());
        assert!(at.elapsed() <= std::time::Duration::from_secs(10));
        writer.abort();
    }
}
