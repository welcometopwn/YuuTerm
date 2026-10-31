//! Real encrypted SSH packets on loopback; ephemeral synthetic secrets only.
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
use yuuterm_core::{Profile, SshAuth};
use yuuterm_ssh::{Engine, LiveSession, Snapshot, Zeroizing};

struct Control {
    mode: AtomicUsize,
    rotate: AtomicBool,
    password_calls: AtomicUsize,
    responses: AtomicUsize,
    secret: Zeroizing<String>,
}
struct Peer {
    c: Arc<Control>,
    channels: HashMap<ChannelId, Channel<server::Msg>>,
}
impl server::Handler for Peer {
    type Error = anyhow::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> anyhow::Result<server::Auth> {
        self.c.password_calls.fetch_add(1, Ordering::SeqCst);
        if self.c.mode.load(Ordering::SeqCst) == 6 {
            anyhow::bail!("fixture transport termination");
        }
        Ok(
            if self.c.mode.load(Ordering::SeqCst) == 0
                && user == "fixture"
                && password == self.c.secret.as_str()
            {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn auth_keyboard_interactive(
        &mut self,
        user: &str,
        _: &str,
        response: Option<server::Response<'_>>,
    ) -> anyhow::Result<server::Auth> {
        let mode = self.c.mode.load(Ordering::SeqCst);
        if mode == 2 {
            return Ok(server::Auth::reject());
        }
        if let Some(mut response) = response {
            self.c.responses.fetch_add(1, Ordering::SeqCst);
            if mode != 5 {
                return Ok(
                    if user == "fixture"
                        && response
                            .next()
                            .is_some_and(|p| p == self.c.secret.as_bytes())
                    {
                        server::Auth::Accept
                    } else {
                        server::Auth::reject()
                    },
                );
            }
        }
        let prompts = if mode == 3 {
            vec![("Password: ".into(), false), ("OTP: ".into(), false)]
        } else if mode == 4 {
            vec![("Password: ".into(), true)]
        } else if mode == 7 {
            vec![("OTP: ".into(), false)]
        } else {
            vec![("Password: ".into(), false)]
        };
        // Deliberately place the secret in untrusted server display strings: they
        // must never be surfaced in engine snapshots/errors.
        Ok(server::Auth::Partial {
            name: self.c.secret.to_string().into(),
            instructions: self.c.secret.to_string().into(),
            prompts: prompts.into(),
        })
    }
    async fn channel_open_session(
        &mut self,
        ch: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> anyhow::Result<()> {
        self.channels.insert(ch.id(), ch);
        reply.accept().await;
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
        s.channel_success(id)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        s.channel_success(id)?;
        s.data(id, b"password-fixture-ready".to_vec())?;
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        _: &str,
        s: &mut server::Session,
    ) -> anyhow::Result<()> {
        s.channel_failure(id)?;
        s.close(id)?;
        Ok(())
    }
}
struct Fixture {
    c: Arc<Control>,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn start(mode: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let c = Arc::new(Control {
            mode: AtomicUsize::new(mode),
            rotate: AtomicBool::new(false),
            password_calls: AtomicUsize::new(0),
            responses: AtomicUsize::new(0),
            secret: Zeroizing::new(format!("ephemeral-{:x}", rand::random::<u128>())),
        });
        let shared = c.clone();
        let keys = [
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
        ];
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result=listener.accept() => {
                        let (socket,_)=result.unwrap();
                        let config=Arc::new(server::Config {keys:vec![keys[usize::from(shared.rotate.load(Ordering::SeqCst))].clone()],auth_rejection_time:Duration::from_millis(1),..Default::default()});
                        let peer=Peer{c:shared.clone(),channels:HashMap::new()};
                        children.spawn(async move { if let Ok(session)=server::run_stream(config,socket,peer).await {let _=session.await;} });
                    }
                    _=children.join_next(), if !children.is_empty()=>{}
                }
            }
        });
        Self { c, port, task }
    }
    fn profile(&self) -> Profile {
        let mut p = Profile {
            id: 1,
            name: "Password fixture".into(),
            host: "127.0.0.1".into(),
            port: self.port,
            username: "fixture".into(),
            ..Default::default()
        };
        p.ssh.auth = SshAuth::Password;
        p
    }
    fn connect(&self, engine: &Engine) -> LiveSession {
        engine
            .connect_with_password(
                self.profile(),
                Arc::new(|| {}),
                Some(Zeroizing::new(self.c.secret.to_string())),
            )
            .unwrap()
    }
}
async fn wait(live: &LiveSession, c: &Control, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let s = live.snapshot();
            assert!(
                !format!("{s:?}").contains(c.secret.as_str()),
                "Secret leaked into snapshot/status"
            );
            if predicate(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("password fixture deadline")
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn password_success_and_keyboard_interactive_fallback() {
    for mode in [0, 1] {
        let f = Fixture::start(mode).await;
        let temp = tempfile::tempdir().unwrap();
        let e = Engine::new(temp.path().join("trust")).unwrap();
        let (live, _rx) = e
            .connect_streaming_with_password(
                f.profile(),
                Arc::new(|| {}),
                Some(Zeroizing::new(f.c.secret.to_string())),
            )
            .unwrap();
        wait(&live, &f.c, |s| s.trust.is_some()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 0);
        live.trust(true);
        let s = wait(&live, &f.c, |s| s.connected || s.finished).await;
        assert!(s.connected && !s.password_rejected);
        assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.c.responses.load(Ordering::SeqCst), mode);
        live.disconnect();
        wait(&live, &f.c, |s| s.finished).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_password_is_distinct_from_transport_missing_or_unsupported_challenges() {
    for mode in [2, 3, 4, 5, 6, 7] {
        let f = Fixture::start(mode).await;
        let temp = tempfile::tempdir().unwrap();
        let e = Engine::new(temp.path().join("trust")).unwrap();
        let live = f.connect(&e);
        wait(&live, &f.c, |s| s.trust.is_some()).await;
        live.trust(true);
        let s = wait(&live, &f.c, |s| s.finished).await;
        assert_eq!(s.password_rejected, mode == 2);
        assert_eq!(f.c.responses.load(Ordering::SeqCst), usize::from(mode == 5));
        assert!(!s.connected);
    }
    let f = Fixture::start(1).await;
    let temp = tempfile::tempdir().unwrap();
    let e = Engine::new(temp.path().join("trust")).unwrap();
    let live = e
        .connect_with_password(
            f.profile(),
            Arc::new(|| {}),
            Some(Zeroizing::new("wrong-synthetic-value".into())),
        )
        .unwrap();
    wait(&live, &f.c, |s| s.trust.is_some()).await;
    live.trust(true);
    let s = wait(&live, &f.c, |s| s.finished).await;
    assert!(s.password_rejected && !s.status.contains("wrong-synthetic-value"));
    let missing = e.connect(f.profile(), Arc::new(|| {})).unwrap();
    let s = wait(&missing, &f.c, |s| s.finished).await;
    assert!(!s.password_rejected && s.status.contains("password required"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_cancelled_and_changed_host_keys_never_receive_passwords() {
    let f = Fixture::start(0).await;
    let temp = tempfile::tempdir().unwrap();
    let e = Engine::new(temp.path().join("trust")).unwrap();
    for accept in [Some(false), None, Some(true)] {
        let live = f.connect(&e);
        wait(&live, &f.c, |s| s.trust.is_some()).await;
        assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 0);
        if let Some(accept) = accept {
            live.trust(accept)
        } else {
            live.disconnect()
        }
        let s = wait(&live, &f.c, |s| s.finished || s.connected).await;
        assert!(!s.password_rejected);
        if accept == Some(true) {
            assert!(s.connected);
            live.disconnect();
            wait(&live, &f.c, |s| s.finished).await;
        }
    }
    assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 1);
    f.c.rotate.store(true, Ordering::SeqCst);
    let changed = f.connect(&e);
    let s = wait(&changed, &f.c, |s| s.finished).await;
    assert!(!s.password_rejected && s.trust.is_none() && !s.connected);
    assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn password_bounds_and_network_failure_do_not_report_refusal() {
    let f = Fixture::start(0).await;
    let temp = tempfile::tempdir().unwrap();
    let e = Engine::new(temp.path().join("trust")).unwrap();
    let error = e
        .connect_with_password(
            f.profile(),
            Arc::new(|| {}),
            Some(Zeroizing::new("x".repeat(16 * 1024 + 1))),
        )
        .err()
        .expect("oversize must fail");
    assert!(error.to_string().contains("16 KiB"));
    assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 0);
    let mut wrong_mode = f.profile();
    wrong_mode.ssh.auth = SshAuth::KeyFile;
    let error = e
        .connect_with_password(
            wrong_mode,
            Arc::new(|| {}),
            Some(Zeroizing::new(f.c.secret.to_string())),
        )
        .err()
        .expect("wrong mode must fail");
    assert!(!format!("{error:#}").contains(f.c.secret.as_str()));
    let live = e
        .connect_with_password(
            f.profile(),
            Arc::new(|| {}),
            Some(Zeroizing::new(String::new())),
        )
        .unwrap();
    wait(&live, &f.c, |s| s.trust.is_some()).await;
    live.trust(true);
    assert!(!wait(&live, &f.c, |s| s.finished).await.password_rejected);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut profile = f.profile();
    profile.port = listener.local_addr().unwrap().port();
    drop(listener);
    let live = e
        .connect_with_password(
            profile,
            Arc::new(|| {}),
            Some(Zeroizing::new(f.c.secret.to_string())),
        )
        .unwrap();
    let s = wait(&live, &f.c, |s| s.finished).await;
    assert!(!s.password_rejected && !s.connected);
    assert_eq!(f.c.password_calls.load(Ordering::SeqCst), 0);
}
