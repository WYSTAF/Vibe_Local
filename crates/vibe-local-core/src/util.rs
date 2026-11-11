//! Shared utilities: time formatting, FNV-1a hashing, hex encoding and
//! subprocess execution with a hard timeout.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// FNV-1a 32-bit hash (offset basis 2166136261, prime 16777619).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

/// Random 32-character hex token from the OS CSPRNG.
pub fn random_token() -> anyhow::Result<String> {
    let mut buf = [0u8; 16];
    getrandom::getrandom(&mut buf).map_err(|e| anyhow::anyhow!("CSPRNG failure: {e}"))?;
    Ok(hex_encode(&buf))
}

/// Constant-time equality for short secrets (tokens).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// ISO-8601 / RFC-3339 UTC timestamp (second precision) without external deps.
pub fn iso8601_now() -> String {
    iso8601_from_unix(unix_now())
}

pub fn iso8601_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Howard Hinnant's `civil_from_days` algorithm (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Run a command capturing stdout, killing it after `timeout`.
/// Returns `None` when the binary is missing, times out, or fails.
pub fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::sync_channel::<String>(1);
    let handle = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = rx.recv_timeout(Duration::from_secs(2)).ok()?;
                let _ = handle.join();
                return if status.success() { Some(out) } else { None };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = handle.join();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

/// Peak / current resident set size of a process in MiB, per platform.
pub fn process_rss_mb(pid: u32) -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let mut peak = None;
        let mut current = None;
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("VmHWM:") {
                peak = v.trim().trim_end_matches("kB").trim().parse::<f64>().ok();
            } else if let Some(v) = line.strip_prefix("VmRSS:") {
                current = v.trim().trim_end_matches("kB").trim().parse::<f64>().ok();
            }
        }
        peak.or(current).map(|kb| kb / 1024.0)
    }
    #[cfg(target_os = "android")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                return v
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|kb| kb / 1024.0);
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let out = run_with_timeout(
            "ps",
            &["-o", "rss=", "-p", &pid.to_string()],
            Duration::from_secs(5),
        )?;
        out.trim().parse::<f64>().ok().map(|kb| kb / 1024.0)
    }
    #[cfg(target_os = "windows")]
    {
        windows_process_rss_mb(pid)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    )))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_process_rss_mb(pid: u32) -> Option<f64> {
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const PROCESS_VM_READ: u32 = 0x0010;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle == 0 {
            return None;
        }
        let mut counters: ProcessMemoryCounters = zeroed();
        counters.cb = size_of::<ProcessMemoryCounters>() as u32;
        let ok = K32GetProcessMemoryInfo(handle, &mut counters, counters.cb);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(counters.peak_working_set_size as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_vectors() {
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
    }

    #[test]
    fn iso8601_known_dates() {
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso8601_from_unix(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x10]), "00ff10");
    }

    #[test]
    fn token_is_random_hex() {
        let a = random_token().unwrap();
        let b = random_token().unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn timeout_kills_slow_process() {
        let out = run_with_timeout("cargo", &["--version"], Duration::from_secs(10));
        assert!(out.is_some());
        let missing = run_with_timeout(
            "definitely-not-a-real-binary-xyz",
            &[],
            Duration::from_secs(1),
        );
        assert!(missing.is_none());
    }
}
