//! SSH tools panel helpers: a fixed capability probe and a bounded folder-size
//! listing. Both run on their own exec channel; nothing is typed into the
//! user's terminal. Script text avoids `'` and `\` so it survives the login
//! shell's single quotes in sh, bash, zsh and fish alike.
use anyhow::{Result, bail, ensure};
use serde::Serialize;

/// Tools whose presence enables commands in the panel.
pub const PROBE_TOOLS: &[&str] = &[
    "htop",
    "sensors",
    "lsblk",
    "numfmt",
    "systemctl",
    "journalctl",
    "docker",
    "apt",
    "dnf",
    "needs-restarting",
    "ufw",
    "nft",
    "ss",
    "ip",
    "last",
    "sudo",
];

/// Groups that grant journal or Docker access without sudo.
pub const PROBE_GROUPS: &[&str] = &["adm", "systemd-journal", "wheel", "docker"];

pub const PROBE_LIMIT: usize = 4096;

pub fn probe_command() -> String {
    format!(
        r#"sh -c 'LC_ALL=C; export LC_ALL; printf "OS %s\n" "$(uname -s)"; printf "SHELL %s\n" "${{SHELL##*/}}"; printf "UID %s\n" "$(id -u)"; for t in {}; do command -v "$t" >/dev/null 2>&1 && printf "TOOL %s\n" "$t"; done; for g in $(id -Gn); do case "$g" in {}) printf "GROUP %s\n" "$g";; esac; done; true'"#,
        PROBE_TOOLS.join(" "),
        PROBE_GROUPS.join("|")
    )
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Probe {
    pub os: String,
    pub shell: String,
    pub uid: Option<u32>,
    pub tools: Vec<String>,
    /// Membership in `PROBE_GROUPS` only.
    pub groups: Vec<String>,
}

pub(crate) fn parse_probe(bytes: &[u8]) -> Result<Probe> {
    ensure!(bytes.len() <= PROBE_LIMIT, "Probe output exceeded 4 KiB");
    let text = std::str::from_utf8(bytes)?;
    let mut probe = Probe {
        os: String::new(),
        shell: String::new(),
        uid: None,
        tools: Vec::new(),
        groups: Vec::new(),
    };
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        ensure!(
            value.len() <= 64 && !value.chars().any(char::is_control),
            "Invalid probe value"
        );
        match key {
            "OS" => probe.os = value.into(),
            "SHELL" => probe.shell = value.into(),
            "UID" => probe.uid = value.parse().ok(),
            // Only names the probe asked for; anything else is ignored.
            "TOOL" if PROBE_TOOLS.contains(&value) && !probe.tools.iter().any(|t| t == value) => {
                probe.tools.push(value.into())
            }
            "GROUP"
                if PROBE_GROUPS.contains(&value) && !probe.groups.iter().any(|g| g == value) =>
            {
                probe.groups.push(value.into())
            }
            _ => {}
        }
    }
    ensure!(!probe.os.is_empty(), "Probe output incomplete");
    Ok(probe)
}

pub const SIZES_LIMIT: usize = 1024 * 1024;
const SIZES_ROWS: usize = 20_000;
/// Rows returned to the UI after sorting by size.
const SIZES_SHOWN: usize = 5_000;

/// Remote paths are absolute, printable and free of backslashes (fish keeps
/// backslash escapes inside single quotes, so they would not round-trip).
pub fn validate_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/') && path.len() <= 4096,
        "Folder sizes need an absolute remote path"
    );
    ensure!(
        !path.chars().any(|c| c.is_control() || c == '\\'),
        "This path contains characters the folder-size view cannot pass safely"
    );
    Ok(())
}

pub fn sizes_command(path: &str) -> Result<String> {
    validate_path(path)?;
    // The path is a positional argument, never part of the script text.
    let quoted = format!("'{}'", path.replace('\'', r"'\''"));
    Ok(format!(
        r#"sh -c 'LC_ALL=C; export LC_ALL; cd -- "$1" || exit 3; printf "DF %s\n" "$(df -Pk . 2>/dev/null | tail -n 1)"; for d in ./* ./.[!.]* ./..?*; do [ -d "$d" ] && [ ! -L "$d" ] && printf "DIR %s\n" "$d"; done; du -x -k -a -d 1 . 2>/dev/null; printf "YUUTERM_DU_EXIT %s\n" "$?"' sh {quoted}"#
    ))
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Filesystem {
    pub name: String,
    pub total_kib: u64,
    pub used_kib: u64,
    pub available_kib: u64,
    pub mount: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SizeEntry {
    pub name: String,
    pub kib: u64,
    pub directory: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct FolderSizes {
    pub path: String,
    pub total_kib: Option<u64>,
    pub filesystem: Option<Filesystem>,
    pub entries: Vec<SizeEntry>,
    /// `du` reported unreadable entries; sizes may be too small.
    pub incomplete: bool,
    /// Output hit the byte/row limit before `du` finished.
    pub truncated: bool,
    /// Smallest rows left out after sorting.
    pub hidden: usize,
    /// Lines that could not be parsed (for example names containing newlines).
    pub skipped: usize,
}

fn parse_filesystem(line: &str) -> Option<Filesystem> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?.to_string();
    let total_kib = fields.next()?.parse().ok()?;
    let used_kib = fields.next()?.parse().ok()?;
    let available_kib = fields.next()?.parse().ok()?;
    let _capacity = fields.next()?;
    let mount = fields.collect::<Vec<_>>().join(" ");
    (!mount.is_empty()).then_some(Filesystem {
        name,
        total_kib,
        used_kib,
        available_kib,
        mount,
    })
}

/// `truncated` is set when the capture stopped at its limit; the exit marker is
/// then missing by design.
pub(crate) fn parse_sizes(path: &str, bytes: &[u8], truncated: bool) -> Result<FolderSizes> {
    ensure!(
        bytes.len() <= SIZES_LIMIT,
        "Folder size output exceeded 1 MiB"
    );
    let text = String::from_utf8_lossy(bytes);
    let mut result = FolderSizes {
        path: path.into(),
        total_kib: None,
        filesystem: None,
        entries: Vec::new(),
        incomplete: false,
        truncated,
        hidden: 0,
        skipped: 0,
    };
    let mut directories = std::collections::HashSet::new();
    let mut exit = None;
    for (index, line) in text.lines().enumerate() {
        if index >= SIZES_ROWS {
            result.truncated = true;
            break;
        }
        if let Some(rest) = line.strip_prefix("DF ") {
            result.filesystem = parse_filesystem(rest);
        } else if let Some(rest) = line.strip_prefix("DIR ./") {
            directories.insert(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("YUUTERM_DU_EXIT ") {
            exit = rest.trim().parse::<u32>().ok();
        } else if let Some((size, name)) = line.split_once('\t')
            && let Ok(kib) = size.parse::<u64>()
        {
            if name == "." {
                result.total_kib = Some(kib);
            } else if let Some(name) = name.strip_prefix("./")
                && !name.is_empty()
                && !name.contains('/')
            {
                result.entries.push(SizeEntry {
                    name: name.into(),
                    kib,
                    directory: false,
                });
            } else {
                result.skipped += 1;
            }
        } else if !line.is_empty() && line != "DF" {
            result.skipped += 1;
        }
    }
    match exit {
        Some(0) => {}
        Some(1) => result.incomplete = true,
        Some(code) => bail!("Folder size listing failed (du exit {code})"),
        None if result.truncated => {}
        None => bail!("Folder not found or not accessible"),
    }
    for entry in &mut result.entries {
        entry.directory = directories.contains(&entry.name);
    }
    result
        .entries
        .sort_by(|a, b| b.kib.cmp(&a.kib).then_with(|| a.name.cmp(&b.name)));
    if result.entries.len() > SIZES_SHOWN {
        result.hidden = result.entries.len() - SIZES_SHOWN;
        result.entries.truncate(SIZES_SHOWN);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_keeps_only_requested_tools() {
        let p = parse_probe(
            b"OS Linux\nSHELL bash\nUID 0\nGROUP docker\nGROUP root\nGROUP adm\nGROUP docker\nTOOL htop\nTOOL docker\nTOOL htop\nTOOL rm\nNOISE x\n",
        )
        .unwrap();
        assert_eq!(p.os, "Linux");
        assert_eq!(p.shell, "bash");
        assert_eq!(p.uid, Some(0));
        assert_eq!(p.tools, ["htop", "docker"]);
        assert_eq!(p.groups, ["docker", "adm"]);
        assert!(parse_probe(b"SHELL bash\n").is_err());
        assert!(parse_probe(b"OS Lin\x1bux\n").is_err());
        assert!(parse_probe(&vec![b'x'; PROBE_LIMIT + 1]).is_err());
    }

    #[test]
    fn probe_script_has_no_characters_the_login_shell_would_reinterpret() {
        let command = probe_command();
        let script = &command["sh -c '".len()..command.len() - 1];
        assert!(!script.contains('\''));
        assert!(!script.contains(r"\\"));
        for tool in PROBE_TOOLS {
            assert!(script.contains(tool));
        }
        assert!(script.contains("adm|systemd-journal|wheel|docker)"));
    }

    #[test]
    fn sizes_path_is_quoted_and_validated() {
        let command = sizes_command("/srv/it's here").unwrap();
        assert!(command.ends_with(r#" sh '/srv/it'\''s here'"#));
        let script = &command["sh -c '".len()..command.find("' sh ").unwrap()];
        assert!(!script.contains('\''));
        for bad in ["relative", "/a\nb", "/a\\b", "/a\u{7f}b", ""] {
            assert!(sizes_command(bad).is_err(), "{bad:?}");
        }
        assert!(sizes_command(&format!("/{}", "a".repeat(4096))).is_err());
    }

    #[test]
    fn sizes_are_sorted_and_directories_marked() {
        let output = "DF /dev/sda1 1000 600 400 60% /\n\
                      DIR ./var\nDIR ./my dir\n\
                      4\t./small.txt\n500\t./var\n70\t./my dir\n9\t./tab\tname\n590\t.\n\
                      YUUTERM_DU_EXIT 0\n";
        let s = parse_sizes("/", output.as_bytes(), false).unwrap();
        assert_eq!(s.total_kib, Some(590));
        assert_eq!(
            s.filesystem,
            Some(Filesystem {
                name: "/dev/sda1".into(),
                total_kib: 1000,
                used_kib: 600,
                available_kib: 400,
                mount: "/".into()
            })
        );
        let names: Vec<_> = s
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.kib, e.directory))
            .collect();
        assert_eq!(
            names,
            [
                ("var", 500, true),
                ("my dir", 70, true),
                ("tab\tname", 9, false),
                ("small.txt", 4, false)
            ]
        );
        assert!(!s.incomplete && !s.truncated && s.skipped == 0);
    }

    #[test]
    fn sizes_report_partial_failed_and_truncated_runs() {
        let partial = parse_sizes("/x", b"DF \n8\t./a\n8\t.\nYUUTERM_DU_EXIT 1\n", false).unwrap();
        assert!(partial.incomplete);
        assert_eq!(partial.filesystem, None);
        assert!(parse_sizes("/x", b"", false).is_err());
        assert!(parse_sizes("/x", b"DF \nYUUTERM_DU_EXIT 2\n", false).is_err());
        let cut = parse_sizes("/x", b"8\t./a\n9\t./b", true).unwrap();
        assert!(cut.truncated);
        assert_eq!(cut.entries.len(), 2);
        // A name with a newline yields an unparseable continuation line.
        let odd = parse_sizes("/x", b"3\t./new\nline\n3\t.\nYUUTERM_DU_EXIT 0\n", false).unwrap();
        assert_eq!(odd.skipped, 1);
        // Non-UTF-8 names are shown lossily instead of failing the listing.
        let lossy = parse_sizes("/x", b"3\t./\xff\n3\t.\nYUUTERM_DU_EXIT 0\n", false).unwrap();
        assert_eq!(lossy.entries[0].name, "\u{fffd}");
    }

    #[test]
    fn sizes_keep_only_the_largest_rows() {
        let mut output = String::new();
        for i in 0..SIZES_SHOWN + 3 {
            output.push_str(&format!("{i}\t./f{i}\n"));
        }
        output.push_str("YUUTERM_DU_EXIT 0\n");
        let s = parse_sizes("/x", output.as_bytes(), false).unwrap();
        assert_eq!(s.hidden, 3);
        assert_eq!(s.entries.len(), SIZES_SHOWN);
        assert_eq!(s.entries[0].kib as usize, SIZES_SHOWN + 2);
    }
}
