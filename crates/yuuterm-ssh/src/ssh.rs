use crate::{
    Command, FileEntry, Snapshot, TrustPrompt, Wake, terminal::Terminal, trust::TrustStore,
};
use anyhow::{Context, Result, bail};
use russh::{
    ChannelMsg, client,
    keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, ssh_key::HashAlg},
};
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::StatusCode,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use yuuterm_core::{Profile, SshAuth};

pub(crate) struct Handler {
    pub x11: Option<Arc<crate::x11::Forward>>,
    pub host: String,
    pub role: String,
    pub store: Arc<Mutex<Result<TrustStore, String>>>,
    pub state: watch::Sender<Snapshot>,
    pub reply: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    pub wake: Wake,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl client::Handler for Handler {
    type Error = anyhow::Error;

    async fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _: &str,
        _: u32,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<()> {
        let Some(forward) = self.x11.clone() else {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };
        let Ok(permit) = forward.permits.clone().try_acquire_owned() else {
            forward.reject();
            reply
                .reject(russh::ChannelOpenFailure::ResourceShortage)
                .await;
            return Ok(());
        };
        if self.cancel.is_cancelled() {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        reply.accept().await;
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if forward.bridge(channel.into_stream(), cancel).await.is_err() {
                forward.reject();
            }
        });
        Ok(())
    }

    async fn check_server_key(&mut self, server: &PublicKeyOrCertificate) -> Result<bool> {
        anyhow::ensure!(
            server.certificate().is_none(),
            "SSH host certificates are not supported yet; connection refused"
        );
        let key = server.public_key();
        let encoded = key.to_openssh()?;
        {
            let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let store = guard.as_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
            if store.check(&self.host, &encoded)? {
                return Ok(true);
            }
        }
        let (tx, rx) = oneshot::channel();
        *self.reply.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        self.state.send_modify(|s| {
            s.status = "Verify the SSH host key".into();
            s.trust = Some(TrustPrompt {
                host: self.host.clone(),
                role: self.role.clone(),
                algorithm: key.algorithm().to_string(),
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            });
        });
        (self.wake)();
        let accepted = tokio::select! {
            _ = self.cancel.cancelled() => false,
            result = tokio::time::timeout(Duration::from_secs(300),rx) => result.ok().and_then(Result::ok).unwrap_or(false),
        };
        self.state.send_modify(|s| {
            s.trust = None;
            s.status = "Authenticating…".into();
        });
        (self.wake)();
        anyhow::ensure!(accepted, "Host key was not approved; connection refused");
        let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_mut()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .remember(&self.host, &encoded)?;
        Ok(true)
    }
}

#[derive(Debug)]
pub(crate) struct PasswordRejected;
impl std::fmt::Display for PasswordRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SSH password rejected; enter the password again")
    }
}
impl std::error::Error for PasswordRejected {}

/// Any failure at a gateway, prefixed "Gateway N (host:port)".
#[derive(Debug)]
pub(crate) struct GatewayError {
    pub index: usize,
    label: String,
    source: anyhow::Error,
}
impl GatewayError {
    pub fn password_rejected(&self) -> bool {
        self.source.is::<PasswordRejected>()
    }
}
impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {:#}", self.label, self.source)
    }
}
impl std::error::Error for GatewayError {}

fn gateway_error(index: usize, label: &str, source: anyhow::Error) -> anyhow::Error {
    GatewayError {
        index,
        label: label.into(),
        source,
    }
    .into()
}

impl Handler {
    fn for_hop(&self, host: &str, port: u16, role: String) -> Self {
        Self {
            x11: None,
            host: format!("[{}]:{}", host.to_ascii_lowercase(), port),
            role,
            store: self.store.clone(),
            state: self.state.clone(),
            reply: self.reply.clone(),
            wake: self.wake.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

/// The authentication view of a gateway hop; no OpenSSH config rules apply.
fn hop_profile(hop: &yuuterm_core::ManualGateway) -> Profile {
    let mut p = Profile {
        host: hop.host.clone(),
        port: hop.port,
        username: hop.username.clone(),
        ..Default::default()
    };
    p.ssh.auth = hop.auth;
    p.ssh.key_file = hop.key_file.clone();
    p.ssh.agent_pipe = hop.agent_pipe.clone();
    p.ssh.identity_fingerprint = hop.identity_fingerprint.clone();
    p
}

async fn handshake<S>(
    config: &Arc<client::Config>,
    stream: S,
    handler: Handler,
) -> Result<client::Handle<Handler>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::time::timeout(
        Duration::from_secs(315),
        client::connect_stream(config.clone(), stream, handler),
    )
    .await
    .context("SSH handshake timed out")?
}

/// A `direct-tcpip` channel from `gateway` to the next hop; the gateway
/// resolves the name, so internal names work.
async fn tunnel(
    gateway: &client::Handle<Handler>,
    host: &str,
    port: u16,
) -> Result<russh::ChannelStream<client::Msg>> {
    let channel = tokio::time::timeout(
        Duration::from_secs(15),
        gateway.channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0),
    )
    .await
    .with_context(|| format!("timed out opening a tunnel to {host}:{port}"))?
    .map_err(|e| anyhow::anyhow!("refused a tunnel to {host}:{port} ({e})"))?;
    Ok(channel.into_stream())
}

async fn password_authenticate(
    session: &mut client::Handle<Handler>,
    username: &str,
    password: Option<zeroize::Zeroizing<String>>,
) -> Result<()> {
    use client::KeyboardInteractiveAuthResponse as Reply;
    let password =
        password.context("SSH password required; desktop credential prompt is not wired yet")?;
    anyhow::ensure!(!password.is_empty(), "SSH password is empty");
    // Host verification has completed in connect_stream before reaching here.
    // russh owns short-lived String copies for its authentication messages. Never
    // format those messages/errors/challenges into application diagnostics.
    let result = session
        .authenticate_password(username, password.as_str())
        .await
        .map_err(|_| anyhow::anyhow!("SSH password authentication transport failed"))?;
    if result.success() {
        return Ok(());
    }
    if matches!(
        result,
        russh::client::AuthResult::Failure {
            partial_success: true,
            ..
        }
    ) {
        bail!("SSH requires additional authentication; multi-factor prompts are unsupported");
    }
    let challenge = session
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|_| anyhow::anyhow!("SSH keyboard-interactive transport failed"))?;
    match challenge {
        Reply::Failure {
            partial_success: false,
            ..
        } => return Err(PasswordRejected.into()),
        Reply::InfoRequest { prompts, .. }
            if prompts.len() == 1
                && !prompts[0].echo
                && matches!(
                    prompts[0].prompt.trim().to_ascii_lowercase().as_str(),
                    "password:" | "password"
                ) => {}
        _ => bail!(
            "SSH requires an unsupported authentication challenge; only one hidden Password prompt is supported"
        ),
    }
    let response = session
        .authenticate_keyboard_interactive_respond(vec![password.to_string()])
        .await
        .map_err(|_| anyhow::anyhow!("SSH keyboard-interactive transport failed"))?;
    match response {
        Reply::Success => Ok(()),
        Reply::Failure {
            partial_success: false,
            ..
        } => Err(PasswordRejected.into()),
        _ => bail!(
            "SSH requires additional authentication; repeated or multi-factor prompts are unsupported"
        ),
    }
}

async fn authenticate(
    session: &mut client::Handle<Handler>,
    profile: &Profile,
    _state: &watch::Sender<Snapshot>,
    password: Option<zeroize::Zeroizing<String>>,
) -> Result<()> {
    if profile.ssh.auth == SshAuth::Password {
        return password_authenticate(session, &profile.username, password).await;
    }
    #[cfg(feature = "startup-probe")]
    let mut probe = crate::startup::Probe::new(_state);
    let hash = session.best_supported_rsa_hash().await?.flatten();
    #[cfg(feature = "startup-probe")]
    probe.mark("rsa_extension_ms");
    match profile.ssh.auth {
        SshAuth::Password => unreachable!("handled before key negotiation"),
        SshAuth::WindowsAgent => {
            #[cfg(windows)]
            {
                let mut agent =
                    crate::agent_pipe::connect(&profile.ssh.agent_pipe, Duration::from_secs(5))
                        .await?;
                #[cfg(feature = "startup-probe")]
                probe.mark("agent_connect_ms");
                let identities = agent
                    .request_identities()
                    .await
                    .context("Could not read agent identities")?;
                #[cfg(feature = "startup-probe")]
                probe.mark("agent_identities_ms");
                anyhow::ensure!(
                    identities.len() <= 64,
                    "Agent returned too many identities; select a smaller key set"
                );
                let keys: Vec<_> = identities
                    .into_iter()
                    .filter_map(|identity| match identity {
                        russh::keys::agent::AgentIdentity::PublicKey { key, .. } => Some(key),
                        _ => None,
                    })
                    .collect();
                let selected_profile = profile.clone();
                let home = std::env::var_os("USERPROFILE")
                    .map(std::path::PathBuf::from)
                    .context("Windows user profile directory is unavailable")?;
                let keys = tokio::task::spawn_blocking(move || {
                    crate::identity::select(&selected_profile, &home, keys)
                })
                .await??;
                #[cfg(feature = "startup-probe")]
                probe.mark("identity_selection_ms");
                let mut tried = 0;
                for key in keys {
                    if matches!(key.algorithm(), russh::keys::Algorithm::Rsa { .. })
                        && hash.is_none()
                    {
                        continue;
                    }
                    if tried >= 6 {
                        bail!(
                            "Authentication stopped after 6 explicitly permitted agent keys; narrow IdentityFile/IdentitiesOnly or select a fingerprint"
                        );
                    }
                    tried += 1;
                    if session
                        .authenticate_publickey_with(&profile.username, key, hash, &mut agent)
                        .await?
                        .success()
                    {
                        #[cfg(feature = "startup-probe")]
                        probe.mark("agent_authentication_ms");
                        return Ok(());
                    }
                }
                bail!(
                    "No selected agent key authenticated ({tried} tried). Check username, agent keys and identity fingerprint"
                );
            }
            #[cfg(not(windows))]
            bail!("Windows named-pipe agent is only available on Windows");
        }
        SshAuth::KeyFile => {
            anyhow::ensure!(
                !profile.ssh.key_file.is_empty(),
                "Select a private-key file in session settings"
            );
            let path = profile.ssh.key_file.clone();
            let key = tokio::task::spawn_blocking(move || -> Result<_> {
                use std::io::Read;
                let f = std::fs::File::open(&path).context("Cannot access the selected private-key file")?;
                anyhow::ensure!(f.metadata()?.is_file(), "Private-key path is not a file");
                let mut encoded = zeroize::Zeroizing::new(String::new());
                f.take(1024*1024+1).read_to_string(&mut encoded)?;
                anyhow::ensure!(encoded.len() <= 1024*1024,"Private-key file exceeds the 1 MiB limit");
                russh::keys::decode_secret_key(&encoded, None).context("Cannot load key. For encrypted keys, load it into the Windows agent; passphrase entry is not implemented yet")
            }).await??;
            anyhow::ensure!(
                !matches!(key.algorithm(), russh::keys::Algorithm::Rsa { .. }) || hash.is_some(),
                "Server does not advertise RSA-SHA2; legacy RSA-SHA1 authentication is disabled"
            );
            anyhow::ensure!(
                session
                    .authenticate_publickey(
                        &profile.username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash)
                    )
                    .await?
                    .success(),
                "Private-key authentication failed; check username and key authorization"
            );
        }
    }
    Ok(())
}

async fn connect_tcp(host: &str, port: u16) -> Result<tokio::net::TcpStream> {
    let stream = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .context("SSH TCP connect timed out")??;
    // russh only applies Config.nodelay in connect(), not generic connect_stream().
    // Configure our owned socket before handing it to the SSH transport.
    stream
        .set_nodelay(true)
        .context("Cannot configure SSH TCP latency policy")?;
    Ok(stream)
}

pub(crate) async fn run(
    (profile, password, hops): (Profile, Option<zeroize::Zeroizing<String>>, Vec<crate::Hop>),
    handler: Handler,
    mut commands: mpsc::Receiver<Command>,
    state: &watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
    (file_tx, file_rx): (
        mpsc::Sender<crate::files::Request>,
        mpsc::Receiver<crate::files::Request>,
    ),
) -> Result<()> {
    let x11 = handler.x11.clone();
    #[cfg(feature = "startup-probe")]
    let mut probe = crate::startup::Probe::new(state);
    let config = config();
    let gateways = connect_gateways(hops, &handler, state, &wake).await?;
    let total = gateways.len();
    let stream = match gateways.last() {
        None => None,
        Some((last, label)) => Some(
            tunnel(last, &profile.host, profile.port)
                .await
                .map_err(|source| gateway_error(total - 1, label, source))?,
        ),
    };
    #[cfg(feature = "startup-probe")]
    probe.mark("tcp_ms");
    let mut connection = match stream {
        None => {
            handshake(
                &config,
                connect_tcp(&profile.host, profile.port).await?,
                handler,
            )
            .await?
        }
        Some(stream) => {
            state.send_modify(|s| s.status = "Connecting…".into());
            handshake(&config, stream, handler).await?
        }
    };
    #[cfg(feature = "startup-probe")]
    probe.mark("handshake_ms");
    tokio::time::timeout(
        Duration::from_secs(30),
        authenticate(&mut connection, &profile, state, password),
    )
    .await
    .context("SSH authentication timed out")??;
    #[cfg(feature = "startup-probe")]
    probe.mark("authentication_ms");
    let connection = Arc::new(connection);
    let mut channel = connection.channel_open_session().await?;
    if let Some(forward) = x11 {
        channel
            .request_x11(true, false, "MIT-MAGIC-COOKIE-1", forward.cookie(), 0)
            .await?;
        request_success(&mut channel)
            .await
            .context("Server rejected X11 forwarding")?;
    }
    channel
        .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
        .await?;
    request_success(&mut channel)
        .await
        .context("PTY request failed")?;
    channel.request_shell(true).await?;
    request_success(&mut channel)
        .await
        .context("Shell request failed")?;
    #[cfg(feature = "startup-probe")]
    probe.mark("pty_shell_ms");
    state.send_modify(|s| {
        s.connected = true;
        s.status = if profile.ssh.auth == SshAuth::Password {
            "Connected · SSH password authentication"
        } else {
            "Connected · SSH agent/key authentication"
        }
        .into();
    });
    wake();

    // A separate bounded browsing queue keeps directory I/O off terminal input/output.
    let mut tasks = tokio::task::JoinSet::new();
    let sftp_state = state.clone();
    let sftp_wake = wake.clone();
    let sftp_connection = connection.clone();
    tasks.spawn(async move {
        let result = async {
            let channel = tokio::time::timeout(
                Duration::from_secs(10),
                sftp_connection.channel_open_session(),
            )
            .await
            .context("SFTP channel open timed out")??;
            sftp_loop(channel, file_rx, &sftp_state, sftp_wake.clone()).await
        }
        .await;
        if let Err(e) = result {
            sftp_state.send_modify(|s| {
                s.files_status = format!("SFTP unavailable: {e:#} · Listing is stale")
            });
            sftp_wake();
        }
    });
    let mut terminal = Terminal::new();
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = false;
    let process_slot = Arc::new(tokio::sync::Semaphore::new(1));
    let stats_slot = Arc::new(tokio::sync::Semaphore::new(1));
    let probe_slot = Arc::new(tokio::sync::Semaphore::new(1));
    let mut folder_cancel: Option<tokio_util::sync::CancellationToken> = None;
    loop {
        tokio::select! {
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if let Some(output)=&output {
                        output.send(data.to_vec()).await.context("Terminal renderer closed")?;
                        wake();
                    } else { terminal.process(&data); dirty = true; }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => { state.send_modify(|s| s.status = format!("Remote process exited ({exit_status})")); }
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            },
            command = commands.recv() => match command {
                Some(Command::Input(bytes)) => {
                    if output.is_none() { terminal.scroll(-500); dirty = true; }
                    channel.data(&bytes[..]).await?;
                }
                Some(Command::Resize { rows,cols }) => {
                    let (rows,cols) = crate::terminal::dimensions(rows,cols);
                    if output.is_none() { terminal.resize(rows,cols); dirty = true; }
                    channel.window_change(cols.into(),rows.into(),0,0).await?;
                }
                Some(Command::Scroll(delta)) => { if output.is_none() { terminal.scroll(delta); dirty = true; } }
                Some(Command::Statistics(reply)) => {
                    if let Ok(slot) = stats_slot.clone().try_acquire_owned() {
                        let connection = connection.clone();
                        tasks.spawn(async move {
                            let _slot = slot;
                            let result = read_statistics(&connection).await.map_err(|e| format!("{e:#}"));
                            let _ = reply.send(result);
                        });
                    } else { let _ = reply.send(Err("Statistics request already in progress".into())); }
                }
                Some(Command::Processes(reply)) => {
                    if let Ok(slot) = process_slot.clone().try_acquire_owned() {
                        let connection = connection.clone();
                        tasks.spawn(async move {
                            let _slot = slot;
                            let result = async {
                                let (data, _) = read_metrics(&connection, MetricCommand::Processes).await?;
                                crate::statistics::parse_processes(&data)
                            }.await.map_err(|e: anyhow::Error| format!("{e:#}"));
                            let _ = reply.send(result);
                        });
                    } else { let _ = reply.send(Err("Process request already in progress".into())); }
                }
                Some(Command::ToolsProbe(reply)) => {
                    if let Ok(slot) = probe_slot.clone().try_acquire_owned() {
                        let connection = connection.clone();
                        tasks.spawn(async move {
                            let _slot = slot;
                            let result = async {
                                let capture = exec_capture(&connection, &crate::tools::probe_command(), crate::tools::PROBE_LIMIT, Duration::from_secs(5), None).await?;
                                anyhow::ensure!(!capture.truncated, "Probe output exceeded 4 KiB");
                                crate::tools::parse_probe(&capture.data)
                            }.await.map_err(|e: anyhow::Error| format!("{e:#}"));
                            let _ = reply.send(result);
                        });
                    } else { let _ = reply.send(Err("Tools probe already in progress".into())); }
                }
                Some(Command::FolderSizes { path, reply }) => {
                    if let Some(previous) = folder_cancel.take() { previous.cancel(); }
                    let cancel = tokio_util::sync::CancellationToken::new();
                    folder_cancel = Some(cancel.clone());
                    let connection = connection.clone();
                    tasks.spawn(async move {
                        let result = async {
                            let command = crate::tools::sizes_command(&path)?;
                            let capture = exec_capture(&connection, &command, crate::tools::SIZES_LIMIT, Duration::from_secs(120), Some(&cancel)).await?;
                            crate::tools::parse_sizes(&path, &capture.data, capture.truncated)
                        }.await.map_err(|e: anyhow::Error| format!("{e:#}"));
                        let _ = reply.send(result);
                    });
                }
                Some(Command::CancelFolderSizes) => {
                    if let Some(previous) = folder_cancel.take() { previous.cancel(); }
                }
                Some(Command::Browse(path)) => {
                    if file_tx.try_send(crate::files::Request::Browse(path)).is_err() { state.send_modify(|s| s.files_status = "SFTP busy or unavailable; path was not opened".into()); wake(); }
                }
                None => break,
            },
            _ = tick.tick(), if dirty => {
                state.send_modify(|s| s.terminal = terminal.view()); dirty = false; wake();
            }
        }
    }
    state.send_modify(|s| s.terminal = terminal.view());
    let _ = connection
        .disconnect(russh::Disconnect::ByApplication, "", "en")
        .await;
    Ok(())
}

async fn read_statistics(
    connection: &client::Handle<Handler>,
) -> Result<crate::statistics::Sample> {
    let (data, request_ms) = read_metrics(connection, MetricCommand::Statistics).await?;
    crate::statistics::parse(&data, request_ms)
}

enum MetricCommand {
    Statistics,
    Processes,
}

async fn read_metrics(
    connection: &client::Handle<Handler>,
    command: MetricCommand,
) -> Result<(Vec<u8>, f64)> {
    let (command, limit) = match command {
        MetricCommand::Statistics => (crate::statistics::COMMAND, 16384),
        MetricCommand::Processes => (crate::statistics::PROCESSES_COMMAND, 4096),
    };
    let started = std::time::Instant::now();
    let mut channel =
        tokio::time::timeout(Duration::from_secs(3), connection.channel_open_session())
            .await
            .context("Statistics channel timed out")??;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        channel.exec(true, command).await?;
        let mut data = Vec::new();
        let mut request_ms = None;
        let mut exit_ok = false;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Success => {
                    request_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                }
                ChannelMsg::Failure => bail!("Server rejected statistics command"),
                ChannelMsg::Data { data: bytes } => {
                    anyhow::ensure!(
                        data.len() + bytes.len() <= limit,
                        "Metrics output limit exceeded"
                    );
                    data.extend_from_slice(&bytes);
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    anyhow::ensure!(exit_status == 0, "Statistics command failed");
                    exit_ok = true;
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        anyhow::ensure!(exit_ok, "Statistics command did not complete");
        Ok((
            data,
            request_ms.context("SSH request acknowledgement missing")?,
        ))
    })
    .await
    .context("Statistics request timed out");
    let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
    result?
}

struct Capture {
    data: Vec<u8>,
    /// Output reached `limit`; the command was stopped early.
    truncated: bool,
}

/// Runs one command on its own exec channel with bounded output, a deadline and
/// optional cancellation. A non-zero exit is left to the caller's parser.
async fn exec_capture(
    connection: &client::Handle<Handler>,
    command: &str,
    limit: usize,
    deadline: Duration,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Capture> {
    let mut channel =
        tokio::time::timeout(Duration::from_secs(3), connection.channel_open_session())
            .await
            .context("Tools channel timed out")??;
    let never = tokio_util::sync::CancellationToken::new();
    let cancel = cancel.unwrap_or(&never);
    let result = tokio::time::timeout(deadline, async {
        channel.exec(true, command).await?;
        let mut data = Vec::new();
        loop {
            let msg = tokio::select! {
                msg = channel.wait() => msg,
                _ = cancel.cancelled() => bail!("Cancelled"),
            };
            match msg {
                Some(ChannelMsg::Failure) => bail!("Server rejected the tools command"),
                Some(ChannelMsg::Data { data: bytes }) => {
                    let room = limit - data.len();
                    if bytes.len() > room {
                        data.extend_from_slice(&bytes[..room]);
                        return Ok(Capture {
                            data,
                            truncated: true,
                        });
                    }
                    data.extend_from_slice(&bytes);
                }
                Some(ChannelMsg::Close) | None => break,
                _ => {}
            }
        }
        Ok(Capture {
            data,
            truncated: false,
        })
    })
    .await
    .context("Tools request timed out")
    .and_then(|r| r);
    // Early stop: ask the server to end the remote process before closing.
    if !result.as_ref().is_ok_and(|c| !c.truncated) {
        let _ =
            tokio::time::timeout(Duration::from_secs(1), channel.signal(russh::Sig::TERM)).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
    result
}

async fn request_success(channel: &mut russh::Channel<client::Msg>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return Ok(()),
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                    bail!("Server rejected the request")
                }
                _ => {}
            }
        }
    })
    .await
    .context("Server request timed out")?
}

async fn sftp_loop(
    mut channel: russh::Channel<client::Msg>,
    mut requests: mpsc::Receiver<crate::files::Request>,
    state: &watch::Sender<Snapshot>,
    wake: Wake,
) -> Result<()> {
    channel.request_subsystem(true, "sftp").await?;
    request_success(&mut channel)
        .await
        .context("SFTP subsystem request failed")?;
    let sftp = RawSftpSession::new_with_config(
        channel.into_stream(),
        russh_sftp::client::Config {
            max_packet_len: 256 * 1024,
            request_timeout_secs: 10,
            ..Default::default()
        },
    );
    let version = sftp.init().await?;
    let posix_rename = version.extensions.contains_key("posix-rename@openssh.com");
    // One read-only lookup per connection; unsupported/restricted servers retain IDs.
    let owners = crate::ownership::load(&sftp, "/etc/passwd").await;
    let groups = crate::ownership::load(&sftp, "/etc/group").await;
    let mut next = Some(".".to_owned());
    'browse: while let Some(path) = match next.take() {
        Some(p) => Some(p),
        None => match requests.recv().await {
            Some(crate::files::Request::Browse(path)) => Some(path),
            Some(crate::files::Request::BrowseWait {
                path,
                cancel,
                reply,
            }) => {
                if cancel.is_cancelled() {
                    let _ = reply.send(Err("Directory request cancelled".into()));
                    continue 'browse;
                }
                match browse_directory(&sftp, &path, &owners, &groups, state, &wake, Some(&cancel))
                    .await
                {
                    Ok(result) => {
                        let _ = reply.send(result);
                    }
                    Err(error) => {
                        let _ = reply.send(Err(format!("{error:#}")));
                        return Err(error);
                    }
                }
                continue 'browse;
            }
            Some(crate::files::Request::Action {
                action,
                cancel,
                reply,
            }) => {
                let quiet = matches!(&action, crate::files::FileAction::Revision { .. });
                if !quiet {
                    state.send_modify(|s| s.files_status = "SFTP operation in progress…".into());
                    wake();
                }
                let changed = action.changes_remote();
                // Whole directory trees get longer than single-file actions.
                let limit = Duration::from_secs(
                    if matches!(&action, crate::files::FileAction::DownloadTree { .. }) {
                        1800
                    } else {
                        300
                    },
                );
                let mut timed_out = false;
                let outcome = tokio::select! {
                    biased;
                    _=cancel.cancelled()=>Err(anyhow::anyhow!("Cancelled; remote result may be uncertain. Refresh before retrying. Partial files may remain.")),
                    result=tokio::time::timeout(limit,crate::files::execute(&sftp,&action,posix_rename,state,&wake))=>match result {Ok(result)=>result,Err(_)=>{timed_out=true;Err(anyhow::anyhow!("SFTP operation timed out; partial files may remain"))}},
                };
                let failed = outcome.is_err();
                let status = match &outcome {
                    Ok(result) => format!("Completed · {} bytes", result.bytes),
                    Err(e) => format!("{e:#}"),
                };
                if !quiet {
                    state.send_modify(|s| s.files_status = status);
                    wake();
                }
                let _ = reply.send(outcome.map_err(|e| format!("{e:#}")));
                // A cancellation may leave an outstanding open/write request. Close
                // this channel instead of reusing potentially unknown handle state.
                if cancel.is_cancelled() || timed_out {
                    bail!(
                        "SFTP operation cancelled/timed out; reconnect to resume file operations"
                    );
                }
                if changed && !failed {
                    Some(state.borrow().path.clone())
                } else {
                    continue 'browse;
                }
            }
            None => None,
        },
    } {
        let _ = browse_directory(&sftp, &path, &owners, &groups, state, &wake, None).await?;
    }
    let _ = sftp.close_session();
    Ok(())
}

async fn browse_directory(
    sftp: &RawSftpSession,
    path: &str,
    owners: &crate::ownership::Names,
    groups: &crate::ownership::Names,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<std::result::Result<String, String>> {
    state.send_modify(|s| s.files_status = "Loading directory…".into());
    wake();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        read_directory(sftp, path, owners, groups),
    )
    .await;
    // Finish/close remote handles before honoring cancellation; never publish a
    // timed-out caller's directory later while it is looking elsewhere.
    if cancel.is_some_and(|c| c.is_cancelled()) {
        state.send_modify(|s| {
            s.files_status = "Directory request cancelled · Previous listing retained".into()
        });
        wake();
        if result.is_err() {
            bail!("Directory request timed out; SFTP closed to release any outstanding handles");
        }
        return Ok(Err("Directory request cancelled".into()));
    }
    let outcome = match result {
        Ok(Ok((path, files))) => {
            let canonical = path.clone();
            state.send_modify(|s| {
                s.path = path;
                s.files_status = format!("{} entries", files.len());
                s.files = Arc::new(files);
            });
            Ok(canonical)
        }
        Ok(Err(e)) => {
            let message = format!("Could not list directory: {e:#} · Previous listing retained");
            state.send_modify(|s| s.files_status = message.clone());
            Err(message)
        }
        Err(_) => {
            bail!("Directory request timed out; SFTP closed to release any outstanding handles")
        }
    };
    wake();
    Ok(outcome)
}

async fn read_directory(
    sftp: &RawSftpSession,
    path: &str,
    owners: &crate::ownership::Names,
    groups: &crate::ownership::Names,
) -> Result<(String, Vec<FileEntry>)> {
    let resolved = sftp
        .realpath(path)
        .await?
        .files
        .into_iter()
        .next()
        .context("Server returned no canonical path")?
        .filename;
    anyhow::ensure!(
        resolved.len() <= 4096 && !resolved.contains('\0'),
        "Invalid resolved path"
    );
    let handle = sftp.opendir(resolved.clone()).await?.handle;
    let result = async {
        let mut files = Vec::new();
        let mut names_size = 0;
        loop {
            match sftp.readdir(handle.clone()).await {
                Ok(batch) => {
                    anyhow::ensure!(
                        !batch.files.is_empty(),
                        "Server returned an empty directory batch"
                    );
                    for f in batch.files {
                        if f.filename == "." || f.filename == ".." {
                            continue;
                        }
                        names_size += f.filename.len();
                        anyhow::ensure!(
                            files.len() < 10000 && names_size <= 1024 * 1024,
                            "Directory exceeds preview limit (10000 entries / 1 MiB names)"
                        );
                        anyhow::ensure!(
                            !f.filename.contains('/') && !f.filename.contains('\0'),
                            "Server returned an invalid filename"
                        );
                        files.push(FileEntry {
                            name: f.filename,
                            directory: f.attrs.is_dir(),
                            symlink: f.attrs.is_symlink(),
                            size: f.attrs.size,
                            permissions: f.attrs.permissions,
                            uid: f.attrs.uid,
                            gid: f.attrs.gid,
                            owner: f.attrs.uid.and_then(|id| owners.get(&id).cloned()),
                            group: f.attrs.gid.and_then(|id| groups.get(&id).cloned()),
                            mtime: f.attrs.mtime,
                        });
                    }
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(e.into()),
            }
        }
        files.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok((resolved, files))
    }
    .await;
    let _ = sftp.close(handle).await;
    result
}

fn config() -> Arc<client::Config> {
    Arc::new(client::Config {
        maximum_packet_size: 32768,
        window_size: 1024 * 1024,
        channel_buffer_size: 16,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    })
}

async fn connect_gateways(
    hops: Vec<crate::Hop>,
    handler: &Handler,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<Vec<(client::Handle<Handler>, String)>> {
    let config = config();
    // Gateways stay connected for the session; dropped after the target. A
    // refused or failed tunnel belongs to the gateway that was asked to open it.
    let mut gateways: Vec<(client::Handle<Handler>, String)> = Vec::with_capacity(hops.len());
    let total = hops.len();
    for (index, hop) in hops.into_iter().enumerate() {
        let crate::Hop {
            gateway: hop,
            password,
        } = hop;
        let label = format!("Gateway {} ({}:{})", index + 1, hop.host, hop.port);
        state
            .send_modify(|s| s.status = format!("Connecting to gateway {} of {total}…", index + 1));
        wake();
        let tunneled = match gateways.last() {
            None => None,
            Some((previous, previous_label)) => Some(
                tunnel(previous, &hop.host, hop.port)
                    .await
                    .map_err(|source| gateway_error(index - 1, previous_label, source))?,
            ),
        };
        let connected = async {
            let next = handler.for_hop(
                &hop.host,
                hop.port,
                format!("Gateway {} of {total}", index + 1),
            );
            let mut connection = match tunneled {
                None => handshake(&config, connect_tcp(&hop.host, hop.port).await?, next).await?,
                Some(stream) => handshake(&config, stream, next).await?,
            };
            state.send_modify(|s| s.status = format!("Authenticating at gateway {}…", index + 1));
            wake();
            tokio::time::timeout(
                Duration::from_secs(30),
                authenticate(&mut connection, &hop_profile(&hop), state, password),
            )
            .await
            .context("SSH authentication timed out")??;
            Ok::<_, anyhow::Error>(connection)
        }
        .await
        .map_err(|source| gateway_error(index, &label, source))?;
        gateways.push((connected, label));
    }
    Ok(gateways)
}

/// Loopback transport owned by one connection incarnation. Channels and their
/// copy buffers are bounded; dropping this future aborts every bridge.
pub(crate) async fn forward(
    profile: Profile,
    hops: Vec<crate::Hop>,
    handler: Handler,
    listener: tokio::net::TcpListener,
    state: &watch::Sender<Snapshot>,
    wake: Wake,
    mut ready: Option<oneshot::Sender<()>>,
) -> Result<()> {
    let gateways = connect_gateways(hops, &handler, state, &wake).await?;
    let (last, label) = gateways
        .last()
        .context("A forward requires an SSH gateway")?;
    if ready.is_none() {
        state.send_modify(|s| {
            s.connected = true;
            s.status = "SSH gateway ready".into();
        });
        wake();
    }
    let mut bridges = tokio::task::JoinSet::new();
    let mut health = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = health.tick() => {
                anyhow::ensure!(!last.is_closed(), "SSH gateway connection closed");
            }
            result = bridges.join_next(), if !bridges.is_empty() => {
                result.context("Gateway bridge disappeared")?.context("Gateway bridge failed")??;
            }
            accepted = listener.accept() => {
                let (mut local, peer) = accepted?;
                if !peer.ip().is_loopback() || bridges.len() >= 4 { continue; }
                local.set_nodelay(true)?;
                let mut remote = tunnel(last, &profile.host, profile.port).await
                    .map_err(|e| gateway_error(gateways.len()-1, label, e))?;
                if let Some(ready) = ready.take() { let _ = ready.send(()); }
                bridges.spawn(async move {
                    // EOF, including an abandoned certificate probe, ends only
                    // this channel. Listener and authenticated chain stay alive.
                    let _ = tokio::io::copy_bidirectional_with_sizes(
                        &mut local, &mut remote, 32768, 32768).await;
                    Ok::<(), anyhow::Error>(())
                });
            }
        }
    }
}

#[cfg(test)]
mod socket_tests {
    #[tokio::test]
    async fn actual_ssh_tcp_stream_disables_nagle_before_russh_handoff() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (client, peer) = tokio::join!(super::connect_tcp("127.0.0.1", port), listener.accept());
        let client = client.unwrap();
        let _peer = peer.unwrap();
        assert!(
            client.nodelay().unwrap(),
            "connect_stream cannot apply russh Config.nodelay to a generic stream"
        );
    }
}
