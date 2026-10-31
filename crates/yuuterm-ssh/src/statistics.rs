//! Opt-in Linux host sampling. Fixed command only; no terminal input is injected.
use anyhow::{Result, ensure};
use serde::Serialize;

pub const COMMAND: &str = r#"sh -c 'LC_ALL=C; export LC_ALL; printf "YUUTERM_STATS_V2\n"; head -n 1 /proc/stat; grep -E "^(MemTotal|MemAvailable):" /proc/meminfo; df -Pk / | tail -n 1; printf "UPTIME "; cat /proc/uptime; printf "HOST "; uname -n; printf "KERNEL "; uname -srm; printf "OS "; grep -m 1 "^PRETTY_NAME=" /etc/os-release 2>/dev/null || printf "PRETTY_NAME=Linux\n"; printf "YUUTERM_NET\n"; head -n 66 /proc/net/dev; printf "YUUTERM_ROUTE\n"; head -n 66 /proc/net/route'"#;

#[derive(Debug, Serialize)]
pub struct Sample {
    pub request_ms: f64,
    pub cpu_total: u64,
    pub cpu_idle: u64,
    pub ram_used_percent: f64,
    pub disk_free_percent: f64,
    pub disk_used_percent: u32,
    pub disk_summary: String,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub uptime_seconds: f64,
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub interface: Option<String>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

pub(crate) fn parse(bytes: &[u8], request_ms: f64) -> Result<Sample> {
    ensure!(bytes.len() <= 16384, "Statistics output exceeded 16 KiB");
    let text = std::str::from_utf8(bytes)?;
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("YUUTERM_STATS_V2"),
        "Linux statistics unavailable (unexpected output)"
    );
    let cpu = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure!(
        cpu.first() == Some(&"cpu") && cpu.len() >= 9,
        "Linux /proc/stat unavailable"
    );
    let counts = cpu[1..9]
        .iter()
        .map(|v| v.parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let total = counts
        .iter()
        .try_fold(0u64, |sum, n| sum.checked_add(*n))
        .ok_or_else(|| anyhow::anyhow!("CPU counter overflow"))?;
    let idle = counts[3]
        .checked_add(counts[4])
        .ok_or_else(|| anyhow::anyhow!("CPU counter overflow"))?;
    let mut memory = std::collections::HashMap::new();
    for _ in 0..2 {
        let fields = lines
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>();
        ensure!(
            fields.len() == 3 && fields[2] == "kB",
            "Linux memory counters unavailable"
        );
        memory.insert(fields[0], fields[1].parse::<u64>()?);
    }
    let mem_total = *memory
        .get("MemTotal:")
        .ok_or_else(|| anyhow::anyhow!("MemTotal missing"))?;
    let available = *memory
        .get("MemAvailable:")
        .ok_or_else(|| anyhow::anyhow!("MemAvailable missing"))?;
    let disk = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    ensure!(disk.len() >= 6, "Root filesystem statistics unavailable");
    let disk_total = disk[1].parse::<u64>()?;
    let disk_free = disk[3].parse::<i64>()?.max(0) as u64;
    let disk_used = disk[2].parse::<u64>()?;
    let disk_used_percent = disk[4].trim_end_matches('%').parse::<u32>()?;
    ensure!(
        disk_used_percent <= 100 && disk_used <= disk_total,
        "Invalid filesystem counters"
    );
    let disk_summary = format!(
        "Filesystem  1K-blocks  Used  Available  Use%  Mounted on\n{}",
        disk.join("  ")
    );
    ensure!(
        disk_summary.len() <= 1024 && !disk_summary.chars().any(|c| c.is_control() && c != '\n'),
        "Invalid filesystem metadata"
    );
    ensure!(
        mem_total > 0 && available <= mem_total && disk_total > 0 && disk_free <= disk_total,
        "Invalid host statistics"
    );
    let uptime_seconds = lines
        .next()
        .and_then(|line| line.strip_prefix("UPTIME "))
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| anyhow::anyhow!("Uptime missing"))?
        .parse::<f64>()?;
    ensure!(
        uptime_seconds.is_finite() && (0.0..=3_155_760_000.0).contains(&uptime_seconds),
        "Invalid uptime"
    );
    let metadata = |line: Option<&str>, prefix: &str| -> Result<String> {
        let text = line
            .and_then(|l| l.strip_prefix(prefix))
            .ok_or_else(|| anyhow::anyhow!("Host metadata missing"))?
            .trim();
        ensure!(
            !text.is_empty() && text.len() <= 256 && !text.chars().any(char::is_control),
            "Invalid host metadata"
        );
        Ok(text.to_string())
    };
    let hostname = metadata(lines.next(), "HOST ")?;
    let kernel = metadata(lines.next(), "KERNEL ")?;
    let os = metadata(lines.next(), "OS PRETTY_NAME=")?
        .trim_matches(['\'', '"'])
        .to_string();
    ensure!(
        lines.next() == Some("YUUTERM_NET"),
        "Network marker missing"
    );
    let mut interfaces = Vec::new();
    let mut routes = false;
    for line in lines.by_ref().take(67) {
        if line == "YUUTERM_ROUTE" {
            routes = true;
            break;
        }
        if let Some((name, fields)) = line.split_once(':') {
            let name = name.trim();
            let values = fields.split_whitespace().collect::<Vec<_>>();
            if name != "lo"
                && !name.is_empty()
                && name.len() <= 64
                && !name.chars().any(|c| c.is_whitespace() || c.is_control())
                && values.len() == 16
                && let (Ok(rx), Ok(tx)) = (values[0].parse::<u64>(), values[8].parse::<u64>())
            {
                interfaces.push((name.to_string(), rx, tx));
            }
        }
    }
    let default = if routes {
        lines
            .take(66)
            .filter_map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() >= 8
                    && fields[1] == "00000000"
                    && fields[7] == "00000000"
                    && u32::from_str_radix(fields[3], 16).is_ok_and(|flags| flags & 1 != 0)
                {
                    Some((fields[0].to_string(), fields[6].parse::<u64>().ok()?))
                } else {
                    None
                }
            })
            .min_by_key(|(_, metric)| *metric)
            .map(|(name, _)| name)
    } else {
        None
    };
    let interface = default
        .as_ref()
        .and_then(|name| interfaces.iter().find(|(n, _, _)| n == name))
        .or_else(|| interfaces.first());
    let ram_total_bytes = mem_total
        .checked_mul(1024)
        .ok_or_else(|| anyhow::anyhow!("Memory counter overflow"))?;
    let ram_used_bytes = (mem_total - available)
        .checked_mul(1024)
        .ok_or_else(|| anyhow::anyhow!("Memory counter overflow"))?;
    Ok(Sample {
        request_ms,
        cpu_total: total,
        cpu_idle: idle,
        ram_used_percent: 100.0 * (1.0 - available as f64 / mem_total as f64),
        disk_free_percent: 100.0 * disk_free as f64 / disk_total as f64,
        disk_used_percent,
        disk_summary,
        ram_total_bytes,
        ram_used_bytes,
        uptime_seconds,
        hostname,
        os,
        kernel,
        interface: interface.map(|(name, _, _)| name.clone()),
        rx_bytes: interface.map(|(_, rx, _)| *rx),
        tx_bytes: interface.map(|(_, _, tx)| *tx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_linux_counters_without_double_counting_guest_and_rejects_bad_output() {
        let input = b"YUUTERM_STATS_V2\ncpu 10 0 20 60 10 0 0 0 8 0\nMemTotal: 1000 kB\nMemAvailable: 250 kB\n/dev/root 1000 600 350 64% /\nUPTIME 259200.5 1.0\nHOST fixture-host\nKERNEL Linux 6.8.0 x86_64\nOS PRETTY_NAME=\"Debian GNU/Linux 13\"\nYUUTERM_NET\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n eth1: 3000 0 0 0 0 0 0 0 4000 0 0 0 0 0 0 0\nYUUTERM_ROUTE\neth1 00000000 0100007F 0003 0 0 100 00000000\n";
        let s = parse(input, 12.0).unwrap();
        assert_eq!(s.cpu_total, 100);
        assert_eq!(s.cpu_idle, 70);
        assert_eq!(s.ram_used_percent, 75.0);
        assert_eq!(s.disk_free_percent, 35.0);
        assert_eq!(s.disk_used_percent, 64);
        assert!(s.disk_summary.contains("/dev/root  1000  600  350  64%  /"));
        assert_eq!(s.hostname, "fixture-host");
        assert_eq!(s.os, "Debian GNU/Linux 13");
        assert_eq!(s.uptime_seconds, 259200.5);
        assert_eq!(s.ram_used_bytes, 750 * 1024);
        assert_eq!(s.interface.as_deref(), Some("eth1"));
        assert_eq!(s.rx_bytes, Some(3000));
        assert_eq!(s.tx_bytes, Some(4000));
        assert!(
            parse(
                &String::from_utf8_lossy(input)
                    .replace("259200.5", "NaN")
                    .into_bytes(),
                0.0
            )
            .is_err()
        );
        assert!(parse(b"welcome banner\n", 0.0).is_err());
        assert!(parse(&vec![b'x'; 16385], 0.0).is_err());
        assert!(
            parse(
                &String::from_utf8_lossy(input)
                    .replace("1000 kB", "0 kB")
                    .into_bytes(),
                0.0
            )
            .is_err()
        );
    }
}

/// Requested only on CPU hover/focus, separately from one-second sampling.
pub const PROCESSES_COMMAND: &str =
    r#"sh -c 'LC_ALL=C; export LC_ALL; ps -eo pid=,pcpu=,comm= --sort=-pcpu | head -n 5'"#;

#[derive(Debug, Serialize)]
pub struct Process {
    pub pid: u32,
    pub cpu_percent: f64,
    pub name: String,
}

pub(crate) fn parse_processes(bytes: &[u8]) -> Result<Vec<Process>> {
    ensure!(bytes.len() <= 4096, "Process output exceeded 4 KiB");
    let text = std::str::from_utf8(bytes)?;
    let mut result = Vec::new();
    for line in text.lines() {
        ensure!(result.len() < 5, "Too many process rows");
        let mut fields = line.split_whitespace();
        let pid = fields.next().unwrap_or_default().parse::<u32>()?;
        let cpu_percent = fields.next().unwrap_or_default().parse::<f64>()?;
        let name = fields.collect::<Vec<_>>().join(" ");
        ensure!(
            pid > 0 && cpu_percent.is_finite() && (0.0..=1_000_000.0).contains(&cpu_percent),
            "Invalid process counters"
        );
        ensure!(
            !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control),
            "Invalid process name"
        );
        result.push(Process {
            pid,
            cpu_percent,
            name,
        });
    }
    ensure!(
        !result.is_empty(),
        "Process list unavailable (requires procps ps)"
    );
    Ok(result)
}

#[cfg(test)]
mod process_tests {
    use super::*;
    #[test]
    fn process_rows_are_bounded_and_do_not_accept_control_characters() {
        let p = parse_processes(b" 42 12.5 worker name\n 1 0.0 init\n").unwrap();
        assert_eq!(p[0].pid, 42);
        assert_eq!(p[0].name, "worker name");
        assert_eq!(p[0].cpu_percent, 12.5);
        for bad in [
            "",
            "1 NaN init",
            "0 1 init",
            "1 -1 init",
            "1 2 bad\u{1b}name",
        ] {
            assert!(parse_processes(bad.as_bytes()).is_err());
        }
        assert!(parse_processes("1 0 init\n".repeat(6).as_bytes()).is_err());
        assert!(parse_processes(&vec![b'x'; 4097]).is_err());
    }
}
