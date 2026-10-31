//! Local Windows agent connection policy; signing/framing remain in russh.
use anyhow::{Context, Result};
use russh::keys::agent::client::AgentClient;
use std::time::Duration;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

pub(crate) async fn connect(
    path: &str,
    deadline: Duration,
) -> Result<AgentClient<NamedPipeClient>> {
    anyhow::ensure!(
        path.starts_with(r"\\.\pipe\"),
        "A local Windows SSH agent pipe is required"
    );
    tokio::time::timeout(deadline, async {
        loop {
            match ClientOptions::new().open(path) {
                Ok(stream) => return Ok(AgentClient::connect(stream)),
                // Windows ERROR_PIPE_BUSY. Do not retry missing pipes or denied access.
                Err(e) if e.raw_os_error() == Some(231) => {
                    // Upstream's 50 ms retry becomes a queue of whole quanta with
                    // simultaneous sessions and a single-instance Windows agent.
                    // Yield between attempts; dropping this future cancels all retries.
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Err(e) => return Err(e),
            }
        }
    })
    .await
    .context("Windows SSH agent pipe connection timed out")?
    .context("Windows SSH agent unavailable. Unlock the key provider and check the agent pipe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    fn pipe() -> String {
        format!(
            r"\\.\pipe\yuuterm-pipe-contention-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        )
    }

    #[tokio::test(start_paused = true)]
    async fn busy_pipe_is_retried_before_the_upstream_fifty_ms_quantum() {
        let name = pipe();
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .max_instances(1)
            .create(&name)
            .unwrap();
        let held = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let connecting = tokio::spawn(async move { connect(&name, Duration::from_secs(1)).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(10)).await;
        server.disconnect().unwrap();
        drop(held);
        let released = tokio::time::Instant::now();
        let (connected, accepted) = tokio::join!(connecting, server.connect());
        assert!(connected.unwrap().is_ok());
        accepted.unwrap();
        assert!(
            released.elapsed() < Duration::from_millis(30),
            "Released pipe still waits for the 50 ms retry timer"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn busy_pipe_has_one_deadline_and_dropped_connector_can_be_cancelled() {
        let name = pipe();
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .max_instances(1)
            .create(&name)
            .unwrap();
        let _held = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        assert!(
            connect(&name, Duration::from_millis(15))
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("timed out")
        );
        let task = tokio::spawn(async move { connect(&name, Duration::from_secs(30)).await });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        assert!(connect(&pipe(), Duration::from_secs(30)).await.is_err());
    }
}
