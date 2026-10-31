//! Experimental real RFB transport, separate from desktop rendering.
//! `start` owns networking on a Tokio worker. Production VNC Connect is not wired.
pub use crate::vnc_preflight::TlsAuth;
use anyhow::{Context, Result, bail};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
use tokio_rustls::{TlsConnector, rustls};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;
pub use zeroize::Zeroizing;
mod compression;
mod framebuffer;
mod trust;
pub use framebuffer::{Cursor, Framebuffer};
pub use rustls::RootCertStore;
pub use trust::{CertificateReview, VncTrust};

const SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_CLIPBOARD_BYTES: usize = 16 * 1024;
const PIXEL_FORMAT: [u8; 16] = [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Fresh per connection attempt, including reconnect. Never reuses a live ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionId(u64);
pub struct Options {
    pub host: String,
    pub port: u16,
    /// Explicit application roots; an empty store requires manual leaf pinning.
    /// This backend does not modify or implicitly import Windows trust.
    pub roots: rustls::RootCertStore,
    pub auth: TlsAuth,
}
/// Kept out of Debug/logs; owned buffers are erased on drop.
pub struct Credentials {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}
pub enum Event {
    CredentialsRequired {
        id: SessionId,
    },
    Connected {
        id: SessionId,
        width: u16,
        height: u16,
    },
    Frame {
        id: SessionId,
        framebuffer: Arc<Framebuffer>,
    },
    TrustRequired {
        id: SessionId,
        review: CertificateReview,
    },
    Closed {
        id: SessionId,
        reason: String,
        password_rejected: bool,
    },
}
#[derive(Clone)]
pub enum Input {
    ReleaseAll,
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    Clipboard { text: Zeroizing<String> },
    Resize { width: u16, height: u16 },
}
pub struct Handle {
    pub id: SessionId,
    pub events: mpsc::Receiver<Event>,
    input: mpsc::Sender<Input>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    credentials: mpsc::Sender<Credentials>,
    auth: TlsAuth,
}
impl Handle {
    pub fn provide_credentials(&self, credentials: Credentials) -> Result<()> {
        validate_credentials(self.auth, &credentials)?;
        self.credentials
            .try_send(credentials)
            .map_err(|_| anyhow::anyhow!("VNC credential request expired"))
    }
    pub fn try_input(&self, id: SessionId, input: Input) -> Result<()> {
        match &input {
            Input::Clipboard { text } => validate_clipboard(text)?,
            Input::Resize { width, height } => {
                anyhow::ensure!(
                    *width > 0
                        && *height > 0
                        && *width as usize * *height as usize <= framebuffer::MAX_PIXELS,
                    "VNC resize exceeds limit"
                );
            }
            _ => {}
        }
        anyhow::ensure!(
            id == self.id && !self.cancel.is_cancelled(),
            "Stale VNC session"
        );
        self.input
            .try_send(input)
            .context("VNC input queue full or closed")
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Cancellation drops the TLS socket, including while a queue is full.
    pub async fn shutdown(mut self) -> Result<()> {
        self.cancel.cancel();
        (&mut self.task).await.context("VNC worker panicked")
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Requires a Tokio runtime; does no network or filesystem I/O on the caller.
pub fn start(options: Options, credentials: Credentials, trust: VncTrust) -> Result<Handle> {
    validate_credentials(options.auth, &credentials)?;
    start_inner(options, Some(credentials), trust)
}

/// Prompts only after TLS and endpoint pin validation. No password exists in
/// the worker until the application answers CredentialsRequired explicitly.
pub fn start_interactive(options: Options, trust: VncTrust) -> Result<Handle> {
    start_inner(options, None, trust)
}
fn validate_credentials(auth: TlsAuth, credentials: &Credentials) -> Result<()> {
    anyhow::ensure!(
        credentials.username.len() <= 4096 && credentials.password.len() <= 4096,
        "VNC credentials exceed limit"
    );
    if matches!(auth, TlsAuth::Vnc) {
        anyhow::ensure!(
            credentials.username.is_empty()
                && credentials.password.is_ascii()
                && credentials.password.len() <= 8,
            "VNC auth requires an ASCII password of at most eight bytes and no username"
        );
    }
    Ok(())
}
fn start_inner(
    options: Options,
    credentials: Option<Credentials>,
    trust: VncTrust,
) -> Result<Handle> {
    anyhow::ensure!(
        options.host.len() <= 253 && options.port > 0,
        "Invalid VNC endpoint"
    );
    rustls::pki_types::ServerName::try_from(options.host.clone())
        .context("Invalid VNC server name")?;
    let auth = options.auth;
    let (credential_tx, credential_rx) = mpsc::channel(1);
    let id = SessionId(
        NEXT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| anyhow::anyhow!("VNC session IDs exhausted"))?,
    );
    let (sender, events) = mpsc::channel(2);
    let (input, inputs) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        let verifier = Arc::new(trust::Verifier {
            roots: options.roots.clone(),
            trust,
            endpoint: format!("[{}]:{}", options.host.to_ascii_lowercase(), options.port),
            review: Default::default(),
            changed: Default::default(),
        });
        let result = tokio::select! {
            biased;
            _ = token.cancelled() => return,
            result = run(id, options, (credentials, credential_rx), verifier.clone(), sender.clone(), inputs) => result,
        };
        let review = verifier.review.lock().unwrap().take();
        let event = if let Some(review) = review {
            Event::TrustRequired { id, review }
        } else {
            let error = result.err();
            let password_rejected = error.as_ref().is_some_and(|e| e.is::<PasswordRejected>());
            let reason = if *verifier.changed.lock().unwrap() {
                "VNC CERTIFICATE CHANGED; connection blocked".into()
            } else {
                error
                    .map(|e| close_reason(&e))
                    .unwrap_or_else(|| "VNC closed".into())
            };
            Event::Closed {
                id,
                reason,
                password_rejected,
            }
        };
        tokio::select! { biased; _ = token.cancelled() => {}, _ = sender.send(event) => {} }
    });
    Ok(Handle {
        id,
        events,
        input,
        cancel,
        task,
        credentials: credential_tx,
        auth,
    })
}

async fn run(
    id: SessionId,
    options: Options,
    (credentials, mut credential_rx): (Option<Credentials>, mpsc::Receiver<Credentials>),
    verifier: Arc<trust::Verifier>,
    events: mpsc::Sender<Event>,
    mut inputs: mpsc::Receiver<Input>,
) -> Result<()> {
    let auth = options.auth;
    let mut stream = tokio::time::timeout(SETUP_TIMEOUT, async {
        let stream = TcpStream::connect((options.host.as_str(), options.port))
            .await
            .context("VNC TCP connection failed")?;
        stream.set_nodelay(true)?;
        let stream =
            crate::vnc_preflight::negotiate_auth(stream, CancellationToken::new(), options.auth)
                .await?
                .into_plaintext_stream_for_tls_upgrade();
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
        // Always revalidate the peer certificate on a new incarnation.
        config.resumption = rustls::client::Resumption::disabled();
        let name = rustls::pki_types::ServerName::try_from(options.host)?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(name, stream)
            .await
            .context("VNC TLS validation failed")?;
        // Persist even a CA-validated identity before credentials. Recheck under
        // the same store lock so concurrent first-use decisions cannot replace it.
        let cert = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .context("VNC TLS certificate missing")?;
        verifier
            .trust
            .approve(CertificateReview {
                endpoint: verifier.endpoint.clone(),
                sha256: trust::fingerprint(cert),
                certificate_der: cert.as_ref().to_vec(),
            })
            .await
            .context("VNC certificate pin could not be saved")?;
        Ok::<_, anyhow::Error>(stream)
    })
    .await
    .context("VNC setup deadline exceeded")??;
    let credentials = match credentials {
        Some(credentials) => credentials,
        None => {
            events
                .send(Event::CredentialsRequired { id })
                .await
                .context("VNC event consumer closed")?;
            credential_rx
                .recv()
                .await
                .context("VNC credential prompt cancelled")?
        }
    };
    let (stream, mut frame) = tokio::time::timeout(SETUP_TIMEOUT, async {
        authenticate(&mut stream, auth, credentials).await?;
        stream.write_u8(1).await?; // Shared ClientInit, only after authentication.
        stream.flush().await?;
        let width = stream.read_u16().await?;
        let height = stream.read_u16().await?;
        let frame = Framebuffer::new(width, height)?;
        let mut format = [0; 16];
        stream.read_exact(&mut format).await?;
        // Replace the server's preferred format before accepting any pixels.
        let name_length = stream.read_u32().await? as usize;
        anyhow::ensure!(name_length <= 4096, "VNC desktop name exceeds limit");
        let mut name = vec![0; name_length];
        stream.read_exact(&mut name).await?; // Never forwarded to UI/logs.
        let mut setup = vec![0, 0, 0, 0];
        setup.extend_from_slice(&PIXEL_FORMAT);
        setup.extend_from_slice(&[2, 0, 0, 6]);
        for encoding in [7i32, 16, 1, 0, -308, -239] {
            setup.extend_from_slice(&encoding.to_be_bytes());
        }
        send(&mut stream, &setup).await?;
        Ok::<_, anyhow::Error>((stream, Arc::new(frame)))
    })
    .await
    .context("VNC setup deadline exceeded")??;
    events
        .send(Event::Connected {
            id,
            width: frame.width,
            height: frame.height,
        })
        .await
        .context("VNC event consumer closed")?;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut incremental = false;
    let mut decoder = compression::Decoder::default();
    let mut keys = std::collections::BTreeSet::new();
    // Release buttons where the pointer is, not at the desktop origin; with no
    // pointer input in this incarnation there is nothing to release.
    let mut position = None;
    let mut request_pending = false;
    loop {
        if !request_pending {
            let mut request = vec![3, u8::from(incremental), 0, 0, 0, 0];
            request.extend_from_slice(&frame.width.to_be_bytes());
            request.extend_from_slice(&frame.height.to_be_bytes());
            tokio::time::timeout(Duration::from_secs(2), send(&mut writer, &request))
                .await
                .context("VNC update request deadline exceeded")??;
            request_pending = true;
        }
        let width = frame.width;
        let height = frame.height;
        let screen = frame.resize;
        {
            let update = async {
                // A static desktop can be silent indefinitely. The absolute body
                // deadline starts at the first byte, and is never reset by input.
                let kind = reader.read_u8().await?;
                tokio::time::timeout(
                    UPDATE_TIMEOUT,
                    Arc::make_mut(&mut frame).message(kind, &mut reader, &mut decoder),
                )
                .await
                .context("VNC framebuffer deadline exceeded")?
            };
            tokio::pin!(update);
            loop {
                tokio::select! {
                    result = &mut update => { if result? { request_pending=false; } break; },
                    Some(input) = inputs.recv() => {
                        tokio::time::timeout(Duration::from_secs(2), async {
                            match input {
                                Input::ReleaseAll => {
                                    for keysym in std::mem::take(&mut keys) { write_input(&mut writer, Input::Key {down:false,keysym}, width, height).await?; }
                                    if let Some((x,y)) = position { write_input(&mut writer, Input::Pointer {buttons:0,x,y}, width, height).await?; }
                                }
                                Input::Key {down,keysym} => {
                                    if down { anyhow::ensure!(keys.len()<128 || keys.contains(&keysym), "VNC pressed-key limit"); keys.insert(keysym); } else {keys.remove(&keysym);}
                                    write_input(&mut writer, input, width, height).await?;
                                }
                                Input::Clipboard {text} => write_clipboard(&mut writer,&text).await?,
                                Input::Resize {width,height} => {
                                    if let Some(screen)=screen { write_resize(&mut writer,width,height,screen).await?; }
                                }
                                Input::Pointer {x,y,..} => {
                                    position = Some((x, y));
                                    write_input(&mut writer, input, width, height).await?;
                                }
                            }
                            Ok::<_, anyhow::Error>(())
                        }).await.context("VNC input deadline exceeded")??;
                    }
                }
            }
        }
        if request_pending {
            continue;
        }
        // No partial frame is published after malformed/truncated rectangles.
        events
            .send(Event::Frame {
                id,
                framebuffer: frame.clone(),
            })
            .await
            .context("VNC event consumer closed")?;
        incremental = true;
    }
}

/// Transport failures name what happened instead of the bare I/O error text.
fn close_reason(error: &anyhow::Error) -> String {
    use std::io::ErrorKind;
    let kind = error
        .chain()
        .find_map(|e| e.downcast_ref::<std::io::Error>())
        .map(std::io::Error::kind);
    let lost = match kind {
        Some(ErrorKind::UnexpectedEof) => "the VNC server closed the connection",
        Some(ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted) => {
            "the VNC server reset the connection"
        }
        Some(ErrorKind::TimedOut) => "the network stopped responding",
        _ => return error.to_string(),
    };
    let context = error.to_string();
    if error.chain().count() > 1 {
        format!("Connection lost: {lost} ({context})")
    } else {
        format!("Connection lost: {lost}")
    }
}

#[derive(Debug)]
struct PasswordRejected;
impl std::fmt::Display for PasswordRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VNC authentication refused")
    }
}
impl std::error::Error for PasswordRejected {}

async fn authenticate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    auth: TlsAuth,
    credentials: Credentials,
) -> Result<()> {
    match auth {
        TlsAuth::Plain => {
            stream.write_u32(credentials.username.len() as u32).await?;
            stream.write_u32(credentials.password.len() as u32).await?;
            stream.write_all(credentials.username.as_bytes()).await?;
            stream.write_all(credentials.password.as_bytes()).await?;
        }
        TlsAuth::Vnc => {
            let mut challenge = [0; 16];
            stream.read_exact(&mut challenge).await?;
            let response = vnc_response(credentials.password.as_bytes(), challenge);
            stream.write_all(response.as_ref()).await?;
        }
    }
    stream.flush().await?;
    drop(credentials);
    if stream.read_u32().await? != 0 {
        return Err(PasswordRejected.into());
    }
    Ok(())
}

fn vnc_response(password: &[u8], challenge: [u8; 16]) -> Zeroizing<[u8; 16]> {
    use des::cipher::{BlockCipherEncrypt, KeyInit};
    let mut key = Zeroizing::new([0u8; 8]);
    for (out, byte) in key.iter_mut().zip(password) {
        *out = byte.reverse_bits();
    }
    let cipher = des::Des::new_from_slice(key.as_ref()).expect("eight-byte DES key");
    let mut response = Zeroizing::new(challenge);
    for chunk in response.as_chunks_mut::<8>().0.iter_mut() {
        let mut block = des::cipher::Block::<des::Des>::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
        block.as_mut_slice().zeroize();
    }
    response
}

/// One write per message. Several small writes become separate TLS records,
/// and TigerVNC 1.15 misframes a SetDesktopSize whose header arrives split
/// (readSetDesktopSize returns "done" without consuming it).
async fn send<W: AsyncWrite + Unpin>(writer: &mut W, message: &[u8]) -> std::io::Result<()> {
    writer.write_all(message).await?;
    writer.flush().await
}

async fn write_input<W: AsyncWrite + Unpin>(
    writer: &mut W,
    input: Input,
    width: u16,
    height: u16,
) -> Result<()> {
    match input {
        Input::ReleaseAll => bail!("VNC release requires tracked input state"),
        Input::Key { down, keysym } => {
            let mut message = vec![4, u8::from(down), 0, 0];
            message.extend_from_slice(&keysym.to_be_bytes());
            send(writer, &message).await?;
        }
        Input::Pointer { buttons, x, y } => {
            let mut message = vec![5, buttons];
            message.extend_from_slice(&x.min(width - 1).to_be_bytes());
            message.extend_from_slice(&y.min(height - 1).to_be_bytes());
            send(writer, &message).await?;
        }
        Input::Clipboard { .. } | Input::Resize { .. } => {
            bail!("VNC control requires session policy")
        }
    }
    Ok(())
}

pub fn validate_clipboard(text: &str) -> Result<()> {
    anyhow::ensure!(
        text.len() <= MAX_CLIPBOARD_BYTES * 2 && text.chars().count() <= MAX_CLIPBOARD_BYTES,
        "VNC clipboard exceeds 16 KiB Latin-1 limit"
    );
    anyhow::ensure!(
        text.chars().all(|c| c as u32 <= 255),
        "Classic VNC clipboard supports Latin-1 only"
    );
    Ok(())
}
async fn write_clipboard<W: AsyncWrite + Unpin>(writer: &mut W, text: &str) -> Result<()> {
    validate_clipboard(text)?;
    let mut message = Zeroizing::new(vec![6, 0, 0, 0]);
    message.extend_from_slice(&(text.chars().count() as u32).to_be_bytes());
    message.extend(text.chars().map(|c| c as u8));
    send(writer, &message).await?;
    Ok(())
}
async fn write_resize<W: AsyncWrite + Unpin>(
    writer: &mut W,
    width: u16,
    height: u16,
    screen: framebuffer::Screen,
) -> Result<()> {
    anyhow::ensure!(
        width > 0 && height > 0 && width as usize * height as usize <= framebuffer::MAX_PIXELS,
        "VNC resize exceeds limit"
    );
    let mut message = vec![251, 0];
    message.extend_from_slice(&width.to_be_bytes());
    message.extend_from_slice(&height.to_be_bytes());
    message.extend_from_slice(&[1, 0]);
    message.extend_from_slice(&screen.id.to_be_bytes());
    message.extend_from_slice(&[0; 4]);
    message.extend_from_slice(&width.to_be_bytes());
    message.extend_from_slice(&height.to_be_bytes());
    message.extend_from_slice(&screen.flags.to_be_bytes());
    send(writer, &message).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
