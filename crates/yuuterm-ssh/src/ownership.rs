//! Optional, bounded account-name lookup through the existing SFTP channel.
use russh_sftp::{
    client::RawSftpSession,
    protocol::{FileAttributes, OpenFlags},
};
use std::{collections::HashMap, time::Duration};

pub(crate) type Names = HashMap<u32, String>;

fn parse_accounts(bytes: &[u8]) -> Names {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Names::new();
    };
    let mut names = Names::new();
    for line in text.lines().take(8192) {
        let mut fields = line.split(':');
        let Some(name) = fields.next() else { continue };
        let _password = fields.next();
        let Some(id) = fields.next().and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        if name.is_empty()
            || name.len() > 256
            || name.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            continue;
        }
        names.entry(id).or_insert_with(|| name.to_owned());
    }
    names
}

pub(crate) async fn load(sftp: &RawSftpSession, path: &str) -> Names {
    let Ok(Ok(opened)) = tokio::time::timeout(
        Duration::from_secs(2),
        sftp.open(path, OpenFlags::READ, FileAttributes::default()),
    )
    .await
    else {
        return Names::new();
    };
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut bytes = Vec::new();
        loop {
            match sftp.read(&opened.handle, bytes.len() as u64, 16384).await {
                Ok(chunk) if !chunk.data.is_empty() => {
                    if bytes.len() + chunk.data.len() > 256 * 1024 {
                        return Names::new();
                    }
                    bytes.extend_from_slice(&chunk.data);
                }
                Ok(_) => return parse_accounts(&bytes),
                Err(russh_sftp::client::error::Error::Status(status))
                    if status.status_code == russh_sftp::protocol::StatusCode::Eof =>
                {
                    return parse_accounts(&bytes);
                }
                Err(_) => return Names::new(),
            }
        }
    })
    .await
    .unwrap_or_default();
    let _ = tokio::time::timeout(Duration::from_secs(2), sftp.close(opened.handle)).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_names_and_unknown_ids_are_distinct() {
        let names = parse_accounts(b"root:x:0:0:root:/root:/bin/zsh\nfixture:x:1000:1000::/:/bin/sh\nbad name:x:42:\nother:x:0:\n");
        assert_eq!(names.get(&0).map(String::as_str), Some("root"));
        assert_eq!(names.get(&1000).map(String::as_str), Some("fixture"));
        assert!(!names.contains_key(&42));
        assert!(!names.contains_key(&999));
        assert!(parse_accounts(&[0xff]).is_empty());
    }
}
