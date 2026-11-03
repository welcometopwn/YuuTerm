#[cfg(windows)]
mod agent_pipe;
pub mod files;
pub mod hex;
mod identity;
pub mod local;
mod ownership;
mod serial;
mod ssh;
#[cfg(feature = "startup-probe")]
mod startup;
pub mod statistics;
mod telnet;
pub mod terminal;
pub mod tools;
mod trust;
pub mod vnc;
pub mod vnc_preflight;
pub mod x11;

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use yuuterm_core::{Profile, Protocol};

pub use zeroize::Zeroizing;

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Enumeration does not open a port or change device control lines.
pub fn serial_ports() -> anyhow::Result<Vec<String>> {
    let mut names: Vec<_> = serialport::available_ports()?
        .into_iter()
        .map(|p| p.port_name)
        .filter(|name| {
            name.strip_prefix("COM").is_some_and(|s| {
                s.bytes().all(|b| b.is_ascii_digit())
                    && !s.starts_with('0')
                    && s.parse::<u16>().is_ok_and(|n| n > 0)
            })
        })
        .take(256)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TrustPrompt {
    pub host: String,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FileEntry {
    pub name: String,
    pub directory: bool,
    pub symlink: bool,
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub mtime: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    #[cfg(feature = "startup-probe")]
    pub startup: std::collections::BTreeMap<&'static str, f64>,
    pub protocol: Protocol,
    pub endpoint: String,
    pub status: String,
    pub connected: bool,
    pub finished: bool,
    /// True only after a definitive SSH password refusal; discard/re-prompt credentials.
    /// Network, trust, timeout and unsupported challenge failures leave this false.
    pub password_rejected: bool,
    pub trust: Option<TrustPrompt>,
    pub terminal: Arc<terminal::TerminalView>,
    pub path: String,
    pub files: Arc<Vec<FileEntry>>,
    pub files_status: String,
}

#[derive(Debug)]
pub enum Command {
    Input(Vec<u8>),
    Resize {
        rows: u16,
        cols: u16,
    },
    Scroll(i32),
    Browse(String),
    Statistics(oneshot::Sender<Result<statistics::Sample, String>>),
    Processes(oneshot::Sender<Result<Vec<statistics::Process>, String>>),
    ToolsProbe(oneshot::Sender<Result<tools::Probe, String>>),
    /// A new request cancels the previous one for this connection.
    FolderSizes {
        path: String,
        reply: oneshot::Sender<Result<tools::FolderSizes, String>>,
    },
    CancelFolderSizes,
}

pub struct LiveSession {
    pub id: u64,
    tx: mpsc::Sender<Command>,
    state: watch::Receiver<Snapshot>,
    cancel: CancellationToken,
    trust_reply: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    files: files::FileClient,
    x11: Arc<Mutex<Option<Arc<x11::Forward>>>>,
}

impl LiveSession {
    pub fn x11_stats(&self) -> Option<x11::Stats> {
        self.x11
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|x| x.stats())
    }
    pub fn files(&self) -> files::FileClient {
        self.files.clone()
    }
    pub fn changed_snapshot(&mut self) -> Snapshot {
        self.state.borrow_and_update().clone()
    }

    pub fn has_changes(&self) -> bool {
        self.state.has_changed().unwrap_or(true)
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.borrow().clone()
    }
    /// Independent bounded metadata observer; consuming it does not acknowledge
    /// changes on the renderer's receiver or keep a connection incarnation alive.
    pub fn subscribe_snapshot(&self) -> watch::Receiver<Snapshot> {
        self.state.clone()
    }
    pub fn send(&self, command: Command) -> Result<(), &'static str> {
        if matches!(&command, Command::Input(b) if b.len() > 16*1024) {
            return Err("Input exceeds the 16 KiB limit.");
        }
        if matches!(&command, Command::Browse(p) if p.len() > 4096 || p.contains('\0')) {
            return Err("Invalid or oversized path.");
        }
        if matches!(&command, Command::FolderSizes { path, .. } if tools::validate_path(path).is_err())
        {
            return Err("Invalid folder path.");
        }
        self.tx
            .try_send(command)
            .map_err(|_| "Connection is closed or busy; input was not sent.")
    }
    pub fn trust(&self, accept: bool) {
        if let Some(reply) = self
            .trust_reply
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = reply.send(accept);
        }
    }
    pub fn disconnect(&self) {
        self.cancel.cancel();
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub struct Engine {
    runtime: Option<tokio::runtime::Runtime>,
    trust: Arc<Mutex<Result<trust::TrustStore, String>>>,
    next_id: AtomicU64,
}

impl Engine {
    pub fn new(trust_path: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            runtime: Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()?,
            ),
            trust: Arc::new(Mutex::new(
                trust::TrustStore::load(trust_path).map_err(|e| e.to_string()),
            )),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn connect(&self, profile: Profile, wake: Wake) -> anyhow::Result<LiveSession> {
        self.connect_with_password(profile, wake, None)
    }

    /// Bounded raw terminal bytes for a frontend terminal parser. Backpressure is
    /// intentional: a slow renderer must not grow an unbounded IPC backlog.
    pub fn connect_streaming(
        &self,
        profile: Profile,
        wake: Wake,
    ) -> anyhow::Result<(LiveSession, mpsc::Receiver<Vec<u8>>)> {
        self.connect_streaming_with_password(profile, wake, None)
    }

    /// Runtime-only secret, never copied into profiles, snapshots or commands.
    pub fn connect_with_password(
        &self,
        profile: Profile,
        wake: Wake,
        password: Option<Zeroizing<String>>,
    ) -> anyhow::Result<LiveSession> {
        self.connect_inner(profile, wake, None, None, password)
    }

    /// Streaming equivalent of `connect_with_password`; existing callers remain unchanged.
    pub fn connect_streaming_with_password(
        &self,
        profile: Profile,
        wake: Wake,
        password: Option<Zeroizing<String>>,
    ) -> anyhow::Result<(LiveSession, mpsc::Receiver<Vec<u8>>)> {
        let (tx, rx) = mpsc::channel(32);
        Ok((
            self.connect_inner(profile, wake, Some(tx), None, password)?,
            rx,
        ))
    }
    /// Runtime-only opt-in for protocol integration with explicit local credentials.
    pub fn connect_x11(
        &self,
        profile: Profile,
        wake: Wake,
        local: x11::LocalX11,
    ) -> anyhow::Result<LiveSession> {
        self.connect_inner(
            profile,
            wake,
            None,
            Some(Arc::new(x11::Forward::new(local))),
            None,
        )
    }

    fn connect_inner(
        &self,
        profile: Profile,
        wake: Wake,
        output: Option<mpsc::Sender<Vec<u8>>>,
        x11: Option<Arc<x11::Forward>>,
        password: Option<Zeroizing<String>>,
    ) -> anyhow::Result<LiveSession> {
        profile.validate().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            matches!(
                profile.protocol,
                Protocol::Ssh | Protocol::Telnet | Protocol::Serial | Protocol::Local
            ),
            "Only SSH, TELNET, SERIAL and local terminals are implemented"
        );
        let is_ssh = profile.protocol == Protocol::Ssh;
        anyhow::ensure!(
            password.as_ref().is_none_or(|p| p.len() <= 16 * 1024),
            "SSH password exceeds the 16 KiB limit"
        );
        anyhow::ensure!(
            password.is_none() || (is_ssh && profile.ssh.auth == yuuterm_core::SshAuth::Password),
            "Runtime password requires SSH password authentication"
        );
        let discover_x11 = is_ssh && profile.ssh.x11_forward && x11.is_none();
        let is_serial = profile.protocol == Protocol::Serial;
        let is_local = profile.protocol == Protocol::Local;
        let unavailable = format!(
            "File browsing is unavailable for {}",
            profile.protocol.label()
        );
        anyhow::ensure!(is_ssh || x11.is_none(), "X11 forwarding requires SSH");
        anyhow::ensure!(
            !is_ssh || !profile.username.trim().is_empty(),
            "Set the SSH username in session settings first"
        );
        let trust = self.trust.lock().unwrap_or_else(|e| e.into_inner());
        if is_ssh && let Err(e) = &*trust {
            anyhow::bail!("{e}");
        }
        drop(trust);
        let (tx, rx) = mpsc::channel(32);
        let (state_tx, state) = watch::channel(Snapshot {
            #[cfg(feature = "startup-probe")]
            startup: Default::default(),
            protocol: profile.protocol,
            endpoint: if is_ssh {
                format!("{}@{}:{}", profile.username, profile.host, profile.port)
            } else if is_serial {
                profile.host.clone()
            } else if is_local {
                profile.local.shell.label().into()
            } else {
                format!("{}:{}", profile.host, profile.port)
            },
            status: "Connecting…".into(),
            connected: false,
            finished: false,
            password_rejected: false,
            trust: None,
            terminal: terminal::Terminal::new().view(),
            path: String::new(),
            files: Arc::new(Vec::new()),
            files_status: if is_ssh {
                "Waiting for SSH authentication"
            } else {
                &unavailable
            }
            .into(),
        });
        let cancel = CancellationToken::new();
        let (file_tx, file_rx) = mpsc::channel(4);
        let file_client = files::FileClient::new(file_tx.clone(), cancel.clone());
        let trust_reply = Arc::new(Mutex::new(None));
        let live_x11 = Arc::new(Mutex::new(x11.clone()));
        let live = LiveSession {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            tx,
            state,
            cancel: cancel.clone(),
            trust_reply: trust_reply.clone(),
            files: file_client,
            x11: live_x11.clone(),
        };
        let mut handler = ssh::Handler {
            x11,
            cancel: cancel.clone(),
            host: format!("[{}]:{}", profile.host.to_ascii_lowercase(), profile.port),
            store: self.trust.clone(),
            state: state_tx.clone(),
            reply: trust_reply,
            wake: wake.clone(),
        };
        self.runtime
            .as_ref()
            .expect("runtime available")
            .spawn(async move {
                let result = tokio::select! {
                    _ = cancel.cancelled() => Ok(()),
                    result = async {
                        if discover_x11 {
                            let local = x11::desktop::local().await?;
                            let forward = Arc::new(x11::Forward::new(local));
                            *live_x11.lock().unwrap_or_else(|e| e.into_inner()) = Some(forward.clone());
                            handler.x11 = Some(forward);
                        }
                        if is_ssh { ssh::run((profile, password), handler, rx, &state_tx, wake.clone(), output, (file_tx, file_rx)).await }
                        else { drop(handler);drop(file_rx);drop(file_tx);
                            if is_local { local::run(profile,rx,state_tx.clone(),wake.clone(),output,cancel.clone()).await }
                            else if is_serial { serial::run(profile,rx,state_tx.clone(),wake.clone(),output,cancel.clone()).await }
                            else { telnet::run(profile,rx,&state_tx,wake.clone(),output).await }
                        }
                    } => result,
                };
                // Natural EOF must revoke auxiliary operations too, not just an
                // explicit UI disconnect or dropping the LiveSession.
                cancel.cancel();
                state_tx.send_modify(|s| {
                    s.connected = false;
                    s.finished = true;
                    s.trust = None;
                    s.password_rejected = result.as_ref().err().is_some_and(|e| e.is::<ssh::PasswordRejected>());
                    s.status = match result {
                        Ok(()) => "Disconnected".into(),
                        Err(e) => format!("{e:#}"),
                    };
                    s.files_status = if is_ssh { "Disconnected · Listing is stale" } else { &unavailable }.into();
                });
                wake();
            });
        Ok(live)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
