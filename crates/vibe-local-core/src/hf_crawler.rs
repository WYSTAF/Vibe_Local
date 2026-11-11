//! Hugging Face GGUF discovery and download with a strict SSRF guard.
//!
//! Only `https://huggingface.co/` and verified HF CDN hosts are allowed;
//! every URL is DNS-resolved and rejected if any address is loopback,
//! link-local (169.254.0.0/16), RFC1918, ULA, multicast or otherwise
//! non-public. Redirects are followed manually and re-validated per hop.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

pub const TRUSTED_QUANTIZERS: &[&str] = &["bartowski", "unsloth", "ggml-org", "MaziyarPanahi"];

const ALLOWED_HOSTS: &[&str] = &[
    "huggingface.co",
    "cdn-lfs.huggingface.co",
    "cdn-lfs-us-1.huggingface.co",
    "cdn-lfs-us-2.huggingface.co",
    "cdn-lfs-eu-1.huggingface.co",
    "cas-bridge.xethub.hf.co",
];

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredFile {
    pub hf_repo: String,
    pub filename: String,
    pub quant: Option<String>,
    pub size_bytes: u64,
    pub download_url: String,
    pub downloads: u64,
    pub likes: u64,
    pub trusted: bool,
}

pub fn is_allowed_host(host: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h))
}

/// True only for globally routable unicast addresses.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    if ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return false;
    }
    // 100.64.0.0/10 CGNAT, 192.0.0.0/24 IETF, 192.0.2.0/24 TEST-NET-1,
    // 198.18.0.0/15 benchmarking, 198.51.100.0/24, 203.0.113.0/24, 240.0.0.0/4.
    if o[0] == 100 && (o[1] & 0xc0) == 64 {
        return false;
    }
    if o[0] == 192 && o[1] == 0 && (o[2] == 0 || o[2] == 2) {
        return false;
    }
    if o[0] == 198 && (o[1] == 18 || o[1] == 19 || (o[1] == 51 && o[2] == 100)) {
        return false;
    }
    if o[0] == 203 && o[1] == 0 && o[2] == 113 {
        return false;
    }
    if o[0] >= 240 {
        return false;
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let seg = ip.segments();
    // ULA fc00::/7, link-local fe80::/10, IPv4-mapped ::ffff:x.x.x.x.
    if (seg[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    if (seg[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // Documentation 2001:db8::/32.
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return false;
    }
    true
}

/// Validate scheme + host allowlist, then DNS-resolve and reject any
/// non-public address (SSRF / DNS-rebinding defense).
pub fn validate_url(raw: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;
    if url.scheme() != "https" {
        anyhow::bail!("only https URLs are allowed");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;
    if !is_allowed_host(host) {
        anyhow::bail!("host {host} is not an allowed Hugging Face domain");
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("DNS resolution failed for {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        anyhow::bail!("DNS returned no addresses for {host}");
    }
    for addr in &addrs {
        if !is_public_ip(addr.ip()) {
            anyhow::bail!("blocked non-public address {} for host {host}", addr.ip());
        }
    }
    Ok(url)
}

fn agent(timeout_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .redirects(0)
        .user_agent("vibe-local/0.1.0")
        .build()
}

/// GET a validated URL following redirects manually (each hop re-validated).
fn get_validated(url: &str, timeout_secs: u64) -> anyhow::Result<ureq::Response> {
    let mut current = url.to_string();
    let ag = agent(timeout_secs);
    for _ in 0..=MAX_REDIRECTS {
        validate_url(&current)?;
        let resp = ag.get(&current).call().map_err(|e| match e {
            ureq::Error::Status(code, _) => anyhow::anyhow!("HTTP {code} from {current}"),
            other => anyhow::anyhow!("request failed: {other}"),
        })?;
        let status = resp.status();
        if (300..400).contains(&status) {
            let location = resp
                .header("location")
                .ok_or_else(|| anyhow::anyhow!("redirect without Location header"))?;
            let base = Url::parse(&current)?;
            current = base.join(location)?.to_string();
            continue;
        }
        return Ok(resp);
    }
    anyhow::bail!("too many redirects (>{MAX_REDIRECTS})")
}

pub fn get_json(url: &str) -> anyhow::Result<serde_json::Value> {
    let resp = get_validated(url, 30)?;
    Ok(resp.into_json()?)
}

/// Search HF for GGUF repos matching `query`, ranked by downloads.
pub fn search_repos(query: &str, limit: usize) -> anyhow::Result<Vec<(String, u64, u64, bool)>> {
    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit={}",
        urlencode(query),
        limit.min(50)
    );
    let json = get_json(&url)?;
    let mut out = Vec::new();
    if let Some(items) = json.as_array() {
        for item in items {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let downloads = item.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
            let likes = item.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
            let owner = id.split('/').next().unwrap_or("");
            let trusted = TRUSTED_QUANTIZERS
                .iter()
                .any(|q| q.eq_ignore_ascii_case(owner));
            out.push((id.to_string(), downloads, likes, trusted));
        }
    }
    // Trusted quantizers first, then by downloads.
    out.sort_by(|a, b| b.3.cmp(&a.3).then(b.1.cmp(&a.1)));
    Ok(out)
}

/// List `.gguf` files of a repo with sizes and direct download URLs.
pub fn repo_gguf_files(
    repo: &str,
    downloads: u64,
    likes: u64,
) -> anyhow::Result<Vec<DiscoveredFile>> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let json = get_json(&url)?;
    let owner = repo.split('/').next().unwrap_or("");
    let trusted = TRUSTED_QUANTIZERS
        .iter()
        .any(|q| q.eq_ignore_ascii_case(owner));
    let mut out = Vec::new();
    if let Some(items) = json.as_array() {
        for item in items {
            let path = item.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if !path.to_lowercase().ends_with(".gguf") {
                continue;
            }
            let size = item
                .get("lfs")
                .and_then(|l| l.get("size"))
                .and_then(|v| v.as_u64())
                .or_else(|| item.get("size").and_then(|v| v.as_u64()))
                .unwrap_or(0);
            let filename = path.rsplit('/').next().unwrap_or(path).to_string();
            out.push(DiscoveredFile {
                hf_repo: repo.to_string(),
                quant: crate::gguf::quant_from_filename(&filename),
                filename,
                size_bytes: size,
                download_url: format!("https://huggingface.co/{repo}/resolve/main/{path}"),
                downloads,
                likes,
                trusted,
            });
        }
    }
    Ok(out)
}

/// Full discovery: search repos, then enumerate GGUF files of the top repos.
pub fn discover(query: &str, max_repos: usize) -> anyhow::Result<Vec<DiscoveredFile>> {
    let repos = search_repos(query, max_repos)?;
    let mut files = Vec::new();
    for (repo, downloads, likes, _trusted) in repos.into_iter().take(max_repos) {
        match repo_gguf_files(&repo, downloads, likes) {
            Ok(mut f) => files.append(&mut f),
            Err(e) => eprintln!("warning: could not list {repo}: {e}"),
        }
    }
    files.sort_by(|a, b| {
        b.downloads
            .cmp(&a.downloads)
            .then(a.size_bytes.cmp(&b.size_bytes))
    });
    Ok(files)
}

/// Download a validated HF URL to `dest` (atomic temp-file + rename).
/// Returns bytes written. Streams with a 16GB ceiling.
pub fn download(url: &str, dest: &Path) -> anyhow::Result<u64> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let resp = get_validated(url, 600)?;
    let tmp = dest.with_extension("gguf.part");
    let mut reader = resp.into_reader().take(16 * 1024 * 1024 * 1024);
    let mut file = std::fs::File::create(&tmp)?;
    let written = std::io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, dest)?;
    Ok(written)
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ip_classification() {
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("10.0.0.5".parse().unwrap()));
        assert!(!is_public_ip("192.168.1.1".parse().unwrap()));
        assert!(!is_public_ip("172.16.0.1".parse().unwrap()));
        assert!(!is_public_ip("169.254.169.254".parse().unwrap()));
        assert!(!is_public_ip("0.0.0.0".parse().unwrap()));
        assert!(!is_public_ip("224.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("100.64.0.1".parse().unwrap()));
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("fe80::1".parse().unwrap()));
        assert!(!is_public_ip("fd00::1".parse().unwrap()));
        assert!(!is_public_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700::1111".parse().unwrap()));
    }

    #[test]
    fn host_allowlist() {
        assert!(is_allowed_host("huggingface.co"));
        assert!(is_allowed_host("cdn-lfs.huggingface.co"));
        assert!(is_allowed_host("cas-bridge.xethub.hf.co"));
        assert!(!is_allowed_host("evil.com"));
        assert!(!is_allowed_host("huggingface.co.evil.com"));
        assert!(!is_allowed_host("127.0.0.1"));
    }

    #[test]
    fn url_validation_rejects_ssrf() {
        // These fail before DNS (scheme/host checks).
        assert!(validate_url("http://huggingface.co/x").is_err());
        assert!(validate_url("https://evil.com/x").is_err());
        assert!(validate_url("https://127.0.0.1/x").is_err());
        assert!(validate_url("https://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_url("https://10.0.0.1/x").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn url_encoding() {
        assert_eq!(urlencode("qwen2.5 coder"), "qwen2.5%20coder");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }

    #[test]
    fn trusted_quantizer_list() {
        assert!(TRUSTED_QUANTIZERS.contains(&"bartowski"));
        assert!(TRUSTED_QUANTIZERS.contains(&"unsloth"));
        assert!(TRUSTED_QUANTIZERS.contains(&"ggml-org"));
        assert!(TRUSTED_QUANTIZERS.contains(&"MaziyarPanahi"));
    }
}
