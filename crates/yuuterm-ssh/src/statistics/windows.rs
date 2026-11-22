use super::Sample;
use anyhow::{Result, ensure};

const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$p=Get-CimInstance Win32_PerfRawData_PerfOS_Processor -Filter "Name='_Total'"
$o=Get-CimInstance Win32_OperatingSystem
$d=Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='C:'"
'YUUTERM_WINSTATS_V1'
"CPU $($p.Timestamp_Sys100NS) $($p.PercentProcessorTime)"
"MEM $($o.TotalVisibleMemorySize) $($o.FreePhysicalMemory)"
"DISK $([uint64]$d.Size) $([uint64]$d.FreeSpace)"
"UPTIME $([uint64]((Get-Date)-$o.LastBootUpTime).TotalSeconds)"
"HOST $env:COMPUTERNAME"
"KERNEL $($o.Version)"
"OS $($o.Caption)""#;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char
            } else {
                '='
            });
        }
    }
    out
}

pub(crate) fn command() -> String {
    let utf16: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64(&utf16)
    )
}

pub(crate) fn parse(bytes: &[u8], request_ms: f64) -> Result<Sample> {
    ensure!(bytes.len() <= 16384, "Statistics output exceeded 16 KiB");
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text
        .lines()
        .map(str::trim)
        .skip_while(|l| *l != "YUUTERM_WINSTATS_V1");
    ensure!(
        lines.next().is_some(),
        "Windows statistics unavailable (unexpected output)"
    );
    let mut field = |prefix: &str| -> Result<String> {
        let line = lines.next().unwrap_or_default();
        let value = line
            .strip_prefix(prefix)
            .ok_or_else(|| anyhow::anyhow!("Windows statistics: {} missing", prefix.trim()))?
            .trim();
        ensure!(
            value.len() <= 256 && !value.chars().any(char::is_control),
            "Invalid Windows statistics"
        );
        Ok(value.to_string())
    };
    let numbers = |value: String| -> Result<Vec<u64>> {
        Ok(value
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()?)
    };
    let cpu = numbers(field("CPU ")?)?;
    let mem = numbers(field("MEM ")?)?;
    let disk = numbers(field("DISK ")?)?;
    let uptime = numbers(field("UPTIME ")?)?;
    let hostname = field("HOST ")?;
    let kernel = field("KERNEL ")?;
    let os = field("OS ")?;
    ensure!(
        cpu.len() == 2 && mem.len() == 2 && disk.len() == 2 && uptime.len() == 1,
        "Invalid Windows statistics"
    );
    let (mem_total, mem_free) = (mem[0], mem[1]);
    ensure!(
        mem_total > 0 && mem_free <= mem_total && disk[1] <= disk[0] && !hostname.is_empty(),
        "Invalid Windows statistics"
    );
    let disk_used_percent = (100 * (disk[0] - disk[1]))
        .checked_div(disk[0])
        .unwrap_or(0) as u32;
    let kib = |v: u64| {
        v.checked_mul(1024)
            .ok_or_else(|| anyhow::anyhow!("Memory counter overflow"))
    };
    Ok(Sample {
        request_ms,
        // Win32_PerfRawData idle time against the 100 ns timestamp: the UI's
        // 1 - Δidle/Δtotal gives the CPU usage like /proc/stat.
        cpu_total: cpu[0],
        cpu_idle: cpu[1],
        ram_used_percent: 100.0 * (1.0 - mem_free as f64 / mem_total as f64),
        disk_free_percent: if disk[0] == 0 {
            0.0
        } else {
            100.0 * disk[1] as f64 / disk[0] as f64
        },
        disk_used_percent,
        disk_summary: format!("C: {} of {} bytes free", disk[1], disk[0]),
        ram_total_bytes: kib(mem_total)?,
        ram_used_bytes: kib(mem_total - mem_free)?,
        uptime_seconds: uptime[0] as f64,
        hostname,
        os,
        kernel,
        interface: None,
        rx_bytes: None,
        tx_bytes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert!(command().starts_with("powershell.exe -NoLogo"));
    }

    #[test]
    fn parses_windows_output_with_crlf_and_a_shell_banner() {
        let out = b"banner\r\nYUUTERM_WINSTATS_V1\r\nCPU 133000000000 120000000000\r\nMEM 16000000 4000000\r\nDISK 500000000000 125000000000\r\nUPTIME 3600\r\nHOST WINHOST\r\nKERNEL 10.0.26100\r\nOS Microsoft Windows 11 Pro\r\n";
        let s = parse(out, 12.0).unwrap();
        assert_eq!(s.hostname, "WINHOST");
        assert_eq!(s.ram_total_bytes, 16_000_000 * 1024);
        assert!((s.ram_used_percent - 75.0).abs() < 0.01);
        assert_eq!(s.disk_used_percent, 75);
        assert_eq!((s.cpu_total, s.cpu_idle), (133000000000, 120000000000));
        assert_eq!(s.interface, None);
    }

    #[test]
    fn rejects_foreign_or_broken_output() {
        assert!(parse(b"'sh' is not recognized", 1.0).is_err());
        assert!(parse(b"YUUTERM_WINSTATS_V1\r\nCPU x y\r\n", 1.0).is_err());
        assert!(parse(b"YUUTERM_WINSTATS_V1\r\nCPU 1 2\r\nMEM 1 5\r\nDISK 1 1\r\nUPTIME 1\r\nHOST h\r\nKERNEL k\r\nOS o\r\n", 1.0).is_err());
    }
}
