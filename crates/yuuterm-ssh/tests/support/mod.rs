use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey, PublicKey},
    server,
};
mod files_fixture;
pub use files_fixture::{FixtureFile, MemoryFiles, add_links, add_navigation_tree};
use files_fixture::{SftpFixture, fixture_files};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
#[derive(Default)]
pub struct X11Control {
    pub request: Mutex<Option<X11Request>>,
    pub server: Mutex<Option<server::Handle>>,
    pub reject: AtomicBool,
}
pub struct X11Request {
    pub protocol: String,
    pub cookie: String,
    pub screen: u32,
    pub single: bool,
    pub before_pty: bool,
}
struct SshFixture {
    x11: Arc<X11Control>,
    pty_requested: bool,
    stats_count: u64,
    posix: Arc<AtomicBool>,
    key: PublicKey,
    files: MemoryFiles,
    channels: HashMap<ChannelId, Channel<server::Msg>>,
    authenticated: Arc<AtomicUsize>,
    offers: Arc<Mutex<Vec<String>>>,
    pub connections: Arc<AtomicUsize>,
    pub resized: Arc<AtomicBool>,
}

impl Drop for SshFixture {
    fn drop(&mut self) {
        self.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

impl server::Handler for SshFixture {
    type Error = anyhow::Error;
    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> anyhow::Result<server::Auth> {
        self.offers.lock().unwrap().push(
            key.fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                .to_string(),
        );
        Ok(if user == "fixture" && key == &self.key {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> anyhow::Result<server::Auth> {
        if user == "fixture" && key == &self.key {
            self.authenticated.fetch_add(1, Ordering::SeqCst);
            Ok(server::Auth::Accept)
        } else {
            Ok(server::Auth::reject())
        }
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> anyhow::Result<()> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        id: ChannelId,
        command: &[u8],
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        if command == yuuterm_ssh::statistics::PROCESSES_COMMAND.as_bytes() {
            s.channel_success(id)?;
            s.data(
                id,
                b"42 12.5 fixture-worker\n43 8.0 sshd\n44 2.0 systemd\n45 1.5 zsh\n46 0.0 idle\n"
                    .to_vec(),
            )?;
            s.exit_status_request(id, 0)?;
            s.eof(id)?;
            s.close(id)?;
            return Ok(());
        }
        if command != yuuterm_ssh::statistics::COMMAND.as_bytes() {
            s.channel_failure(id)?;
            s.close(id)?;
            return Ok(());
        }
        self.stats_count += 1;
        s.channel_success(id)?;
        let user = 100 + self.stats_count * 20;
        let idle = 500 + self.stats_count * 80;
        let rx = 100_000 + self.stats_count * 2048;
        let tx = 200_000 + self.stats_count * 1024;
        s.data(id, format!("YUUTERM_STATS_V2\ncpu {user} 0 100 {idle} 0 0 0 0 0 0\nMemTotal: 4194304 kB\nMemAvailable: 1048576 kB\n/dev/fixture 1000 600 350 64% /\nUPTIME 259200.5 1.0\nHOST fixture-host\nKERNEL Linux 6.8.0 x86_64\nOS PRETTY_NAME=\"Debian GNU/Linux 13\"\nYUUTERM_NET\nInter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n eth0: {rx} 0 0 0 0 0 0 0 {tx} 0 0 0 0 0 0 0\nYUUTERM_ROUTE\nIface Destination Gateway Flags RefCnt Use Metric Mask\neth0 00000000 0100007F 0003 0 0 100 00000000\n").into_bytes())?;
        s.exit_status_request(id, 0)?;
        s.eof(id)?;
        s.close(id)?;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        self.pty_requested = true;
        s.channel_success(id)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        *self.x11.server.lock().unwrap() = Some(s.handle());
        s.channel_success(id)?;
        s.data(id, "\x1b[48;5;24m\x1b[97m \u{f120} fixture \x1b[49m\x1b[38;5;24m\u{e0b0}\x1b[0m \u{f07b} ~ \u{e0b1} \u{f126} main\r\n\x1b[1mBold\x1b[0m / \x1b[3mItalic\x1b[0m / \x1b[1;3mBold italic\x1b[0m\r\n\x1b[32mReady\x1b[0m\r\nERROR connection denied\r\nwarning retry 192.0.2.42\r\nsuccess validation passed\r\n\x1b[36mERROR preserves ANSI cyan\x1b[0m\r\nfixture> ".as_bytes())?;
        Ok(())
    }
    async fn x11_request(
        &mut self,
        id: ChannelId,
        single: bool,
        protocol: &str,
        cookie: &str,
        screen: u32,
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        if self.x11.reject.load(Ordering::SeqCst) {
            s.channel_failure(id)?;
            return Ok(());
        }
        *self.x11.request.lock().unwrap() = Some(X11Request {
            protocol: protocol.into(),
            cookie: cookie.into(),
            screen,
            single,
            before_pty: !self.pty_requested,
        });
        s.channel_success(id)?;
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        if self.channels.contains_key(&id) {
            s.data(id, data.to_vec())?;
        }
        Ok(())
    }
    async fn window_change_request(
        &mut self,
        _: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut server::Session,
    ) -> anyhow::Result<()> {
        if cols == 100 && rows == 30 {
            self.resized.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        if name != "sftp" {
            s.channel_failure(id)?;
            return Ok(());
        }
        s.channel_success(id)?;
        let channel = self.channels.remove(&id).unwrap();
        russh_sftp::server::run(
            channel.into_stream(),
            SftpFixture::new(self.files.clone(), self.posix.load(Ordering::SeqCst)),
        )
        .await;
        Ok(())
    }
}

pub struct Fixture {
    pub x11: Arc<X11Control>,
    pub posix: Arc<AtomicBool>,
    pub files: MemoryFiles,
    task: tokio::task::JoinHandle<()>,
    pub port: u16,
    pub rotate: Arc<AtomicBool>,
    pub auth: Arc<AtomicUsize>,
    pub offers: Arc<Mutex<Vec<String>>>,
    pub connections: Arc<AtomicUsize>,
    pub resized: Arc<AtomicBool>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub async fn start(accepted_key: PublicKey) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let keys = [
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
        ];
        let rotate = Arc::new(AtomicBool::new(false));
        let auth = Arc::new(AtomicUsize::new(0));
        let offers = Arc::new(Mutex::new(Vec::new()));
        let shared_offers = offers.clone();
        let resized = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let (r, a, z, c) = (
            rotate.clone(),
            auth.clone(),
            resized.clone(),
            connections.clone(),
        );
        let files = fixture_files();
        let posix = Arc::new(AtomicBool::new(true));
        let posix_shared = posix.clone();
        let shared_files = files.clone();
        let x11 = Arc::new(X11Control::default());
        let x11_shared = x11.clone();
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept()=>{
                        let (socket,_)=result.unwrap();
                        let config=Arc::new(server::Config {keys:vec![keys[usize::from(r.load(Ordering::SeqCst))].clone()],auth_rejection_time:Duration::from_millis(1),..Default::default()});
                        c.fetch_add(1,Ordering::SeqCst);
                        let handler=SshFixture {offers:shared_offers.clone(),x11:x11_shared.clone(),pty_requested:false,stats_count:0,posix:posix_shared.clone(),files:shared_files.clone(),key:accepted_key.clone(),channels:HashMap::new(),authenticated:a.clone(),connections:c.clone(),resized:z.clone()};
                        children.spawn(async move {if let Ok(session)=server::run_stream(config,socket,handler).await {let _=session.await;}});
                    }
                    _=children.join_next(),if !children.is_empty()=>{}
                }
            }
        });
        Self {
            x11,
            posix,
            files,
            task,
            port,
            rotate,
            auth,
            offers,
            connections,
            resized,
        }
    }
}
