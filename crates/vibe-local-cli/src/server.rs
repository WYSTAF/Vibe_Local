//! Vibe-Link HTTP server: OpenAI-compatible `/v1/*` routes for Vibe Router,
//! `/api/*` control routes, the web leaderboard and strict security defaults
//! (loopback bind, LAN token auth, CSRF content-type guard, CSP).

use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server};

use vibe_local_core::catalog::{self, CatalogEntry};
use vibe_local_core::db::SharedDb;
use vibe_local_core::hardware::HardwareProbe;
use vibe_local_core::{hf_crawler, recommender, util};
use vibe_local_engine::{Engine, LoadRequest};

pub const DEFAULT_PORT: u16 = 20129;
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
const WORKER_THREADS: usize = 4;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub lan: bool,
    pub token: Option<String>,
    pub db: SharedDb,
    pub engine: Arc<Engine>,
    pub web_root: Option<PathBuf>,
    pub catalog_path: Option<PathBuf>,
}

pub struct ServerHandle {
    pub port: u16,
    pub lan: bool,
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl ServerHandle {
    pub fn shutdown(&self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wake blocked recv() loops with one dummy request per worker.
        for _ in 0..WORKER_THREADS {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port)) {
                let _ =
                    stream.write_all(b"GET /__vibe_shutdown HTTP/1.1\r\nHost: localhost\r\n\r\n");
            }
        }
        for handle in self.workers.lock().unwrap().drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start_server(config: ServerConfig) -> anyhow::Result<Arc<ServerHandle>> {
    let bind: SocketAddr = if config.lan {
        ([0, 0, 0, 0], config.port).into()
    } else {
        ([127, 0, 0, 1], config.port).into()
    };
    let server = Server::http(bind).map_err(|e| anyhow::anyhow!("cannot bind {bind}: {e}"))?;
    let actual_port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
    };
    let server = Arc::new(server);
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(AppState {
        config: config.clone(),
        stop: Arc::clone(&stop),
    });
    let mut workers = Vec::new();
    for i in 0..WORKER_THREADS {
        let server = Arc::clone(&server);
        let state = Arc::clone(&state);
        let stop = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name(format!("vibe-http-{i}"))
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match server.recv() {
                        Ok(request) => handle_request(request, &state),
                        Err(_) => break,
                    }
                }
            })?;
        workers.push(handle);
    }
    Ok(Arc::new(ServerHandle {
        port: actual_port,
        lan: config.lan,
        stop,
        workers: Mutex::new(workers),
    }))
}

struct AppState {
    config: ServerConfig,
    stop: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn json_response(
    status: u16,
    value: serde_json::Value,
    extra: &[(&str, String)],
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let mut resp = Response::from_data(body)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Content-Security-Policy", CSP))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    for (k, v) in extra {
        resp = resp.with_header(header(k, v));
    }
    resp
}

fn ok_json(value: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(200, value, &[])
}

fn err_json(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, json!({"ok": false, "error": message}), &[])
}

fn read_body(request: &mut Request) -> anyhow::Result<String> {
    let mut buf = String::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES)
        .read_to_string(&mut buf)?;
    Ok(buf)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(urldecode(v));
        }
    }
    None
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Security middleware
// ---------------------------------------------------------------------------

fn request_header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn authorized(request: &Request, state: &AppState) -> bool {
    if !state.config.lan {
        return true; // loopback-only bind: local trust boundary
    }
    let Some(expected) = state.config.token.as_deref() else {
        return false; // LAN without a token is never allowed
    };
    let provided = request_header(request, "X-Vibe-Local-Token").or_else(|| {
        request_header(request, "Authorization")
            .and_then(|v| v.strip_prefix("Bearer ").map(|t| t.to_string()))
    });
    match provided {
        Some(token) => util::constant_time_eq(token.as_bytes(), expected.as_bytes()),
        None => false,
    }
}

/// State-mutating routes require JSON content-type or the admin header.
fn csrf_ok(request: &Request) -> bool {
    let ct = request_header(request, "Content-Type").unwrap_or_default();
    ct.to_lowercase().starts_with("application/json")
        || request_header(request, "X-Vibe-Admin").as_deref() == Some("1")
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

fn handle_request(mut request: Request, state: &AppState) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    let method = request.method().clone();

    if state.stop.load(Ordering::SeqCst) {
        let _ = request.respond(err_json(503, "server shutting down"));
        return;
    }

    // Static UI is public even in LAN mode (no secrets in the bundle);
    // every API/v1 route is token-gated when LAN is enabled.
    let is_api = path.starts_with("/api/") || path.starts_with("/v1/");
    if is_api && !authorized(&request, state) {
        let _ = request.respond(err_json(
            401,
            "missing or invalid X-Vibe-Local-Token / Bearer token",
        ));
        return;
    }
    let mutating = matches!(method, Method::Post | Method::Delete);
    if is_api && mutating && !csrf_ok(&request) {
        let _ = request.respond(err_json(
            415,
            "state-mutating requests require Content-Type: application/json or X-Vibe-Admin: 1",
        ));
        return;
    }

    let result = route(&mut request, state, &path, &method, &url);
    let response = match result {
        Ok(resp) => resp,
        Err(e) => err_json(500, &format!("{e:#}")),
    };
    let _ = request.respond(response);
}

type HttpResult = anyhow::Result<Response<std::io::Cursor<Vec<u8>>>>;

fn route(
    request: &mut Request,
    state: &AppState,
    path: &str,
    method: &Method,
    url: &str,
) -> HttpResult {
    match (method.clone(), path) {
        (Method::Get, "/api/hardware") => {
            let report = HardwareProbe::scan();
            let db = state.config.db.lock().unwrap();
            db.insert_scan(&report)?;
            Ok(ok_json(serde_json::to_value(&report)?))
        }
        (Method::Get, "/api/recommendations") => {
            let report = HardwareProbe::scan();
            let rec = recommender::recommend(&report);
            Ok(ok_json(serde_json::to_value(&rec)?))
        }
        (Method::Get, "/api/status") => {
            let status = state.config.engine.status()?;
            Ok(ok_json(json!({
                "ok": true,
                "loaded_model": status.loaded_model,
                "backend": status.backend,
                "engine": status.engine,
                "idle_timeout_secs": status.idle_timeout_secs,
                "thermal": {"celsius": status.thermal_celsius, "state": status.thermal_state},
                "lan": state.config.lan,
                "port": state.config.port,
            })))
        }
        (Method::Get, "/api/leaderboard") => {
            let profile = query_param(url, "profile").unwrap_or_default();
            Ok(ok_json(leaderboard_payload(state, &profile)?))
        }
        (Method::Post, "/api/leaderboard/submit") => {
            let body = read_body(request)?;
            let value: serde_json::Value = serde_json::from_str(&body)?;
            let entry_value = value.get("entry").cloned().unwrap_or(value);
            let mut entry: CatalogEntry = serde_json::from_value(entry_value)?;
            if entry.provenance == "seed_estimate" {
                return Ok(err_json(
                    400,
                    "seed_estimate entries cannot be submitted via the API",
                ));
            }
            if entry.provenance != "measured" {
                entry.provenance = "submitted".into();
            }
            entry.validate()?;
            let db = state.config.db.lock().unwrap();
            db.upsert_catalog_entry(&entry)?;
            for (pillar, score) in &entry.pillars {
                db.insert_benchmark(
                    &entry.model_id,
                    &entry.hf_repo,
                    &entry.filename,
                    &entry.quant,
                    &entry.hardware_profile,
                    &entry.thr_throughput_backend(),
                    pillar,
                    score.score,
                    &json!({"metric": score.metric, "n": score.n, "detail": score.detail}),
                    &entry.provenance,
                )?;
            }
            Ok(ok_json(
                json!({"ok": true, "model_id": entry.model_id, "provenance": entry.provenance}),
            ))
        }
        (Method::Get, "/api/models") => {
            let query = query_param(url, "query").unwrap_or_default();
            let db = state.config.db.lock().unwrap();
            Ok(ok_json(
                json!({"ok": true, "results": db.models_for_query(&query)?}),
            ))
        }
        (Method::Get, "/api/discover") => {
            let query = query_param(url, "query").unwrap_or_else(|| "gguf".into());
            let files = hf_crawler::discover(&query, 5)?;
            let db = state.config.db.lock().unwrap();
            for f in &files {
                db.upsert_model(f)?;
            }
            Ok(ok_json(json!({"ok": true, "results": files})))
        }
        (Method::Post, "/api/load") => {
            let body = read_body(request)?;
            let req: LoadRequest = serde_json::from_str(&body)?;
            state.config.engine.load(req)?;
            Ok(ok_json(json!({"ok": true})))
        }
        (Method::Post, "/api/unload") => {
            state.config.engine.unload();
            Ok(ok_json(json!({"ok": true})))
        }
        (Method::Post, "/api/idle") => {
            let body = read_body(request)?;
            let value: serde_json::Value = serde_json::from_str(&body)?;
            let secs = value["seconds"].as_u64().unwrap_or(300);
            state.config.engine.set_idle_secs(secs);
            Ok(ok_json(json!({"ok": true, "idle_timeout_secs": secs})))
        }
        (Method::Post, "/api/thermal") => {
            let body = read_body(request)?;
            let value: serde_json::Value = serde_json::from_str(&body)?;
            let celsius = value["celsius"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("celsius required"))?;
            state.config.engine.thermal().set_override(celsius);
            Ok(ok_json(json!({"ok": true})))
        }
        (Method::Post, "/api/vibemark") => {
            let body = read_body(request)?;
            let value: serde_json::Value = if body.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&body)?
            };
            if let Some(path) = value["path"].as_str() {
                let status = state.config.engine.status()?;
                if status.loaded_model.as_deref() != Some(path) {
                    state.config.engine.load(LoadRequest {
                        path: path.to_string(),
                        backend: value["backend"].as_str().unwrap_or("cpu").to_string(),
                        ngl: value["ngl"].as_u64().unwrap_or(0) as u32,
                        ctx: value["ctx"].as_u64().unwrap_or(4096) as u32,
                    })?;
                }
            }
            let budget = value["budget_secs"].as_u64().unwrap_or(15);
            let result = state.config.engine.vibemark(budget)?;
            let db = state.config.db.lock().unwrap();
            db.insert_vibemark(
                &result.model,
                &result.backend,
                &result.engine,
                result.prefill_tps,
                result.decode_tps,
                result.ttft_ms,
                result.peak_ram_mb,
                result.duration_s,
            )?;
            Ok(ok_json(serde_json::to_value(&result)?))
        }
        (Method::Get, "/api/vibemark/latest") => {
            let db = state.config.db.lock().unwrap();
            Ok(ok_json(
                json!({"ok": true, "result": db.latest_vibemark()?}),
            ))
        }
        (Method::Get, "/v1/models") => {
            let status = state.config.engine.status()?;
            let mut data = Vec::new();
            if let Some(model) = &status.loaded_model {
                let id = std::path::Path::new(model)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| model.clone());
                data.push(json!({
                    "id": id, "object": "model", "created": util::unix_now(),
                    "owned_by": "vibe-local", "backend": status.backend, "engine": status.engine,
                }));
            }
            Ok(ok_json(json!({"object": "list", "data": data})))
        }
        (Method::Post, "/v1/chat/completions") => {
            let body = read_body(request)?;
            let value: serde_json::Value = serde_json::from_str(&body)?;
            let prompt = extract_prompt(&value);
            if prompt.trim().is_empty() {
                return Ok(err_json(400, "messages must contain user content"));
            }
            let strip = value["strip_think"].as_bool() == Some(true)
                || request_header(request, "X-Vibe-Strip-Think").as_deref() == Some("1");
            let max_tokens = value["max_tokens"].as_u64().unwrap_or(512).min(8192) as u32;
            let reply = state.config.engine.chat(&prompt, strip, max_tokens)?;
            let model_id = state
                .config
                .engine
                .status()?
                .loaded_model
                .unwrap_or_else(|| "vibe-local".into());
            let resp = json!({
                "id": format!("chatcmpl-vibe-{}", util::unix_now()),
                "object": "chat.completion",
                "created": util::unix_now(),
                "model": model_id,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": reply.text},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": vibe_local_engine::microeval::tokenize(&prompt).len(),
                    "completion_tokens": reply.tokens_generated,
                    "total_tokens": vibe_local_engine::microeval::tokenize(&prompt).len() as u64 + reply.tokens_generated,
                },
            });
            Ok(json_response(
                200,
                resp,
                &[
                    ("X-Vibe-Router-Cost-Micros", "0".to_string()),
                    (
                        "X-Vibe-Local-Tok-Per-Sec",
                        format!("{:.2}", reply.tok_per_sec),
                    ),
                    ("X-Vibe-Local-Backend", reply.backend.clone()),
                    ("X-Vibe-Local-Engine", reply.engine.clone()),
                ],
            ))
        }
        (Method::Get, "/catalog.json") => Ok(ok_json(catalog_payload(state)?)),
        (Method::Get, "/") | (Method::Get, "/leaderboard") => serve_static(state, "index.html"),
        (Method::Get, p) if !p.contains("..") => {
            let rel = p.trim_start_matches('/');
            if rel.is_empty() {
                return serve_static(state, "index.html");
            }
            serve_static(state, rel)
        }
        _ => Ok(err_json(404, "not found")),
    }
}

trait ThroughputBackend {
    fn thr_throughput_backend(&self) -> String;
}

impl ThroughputBackend for CatalogEntry {
    fn thr_throughput_backend(&self) -> String {
        match self.hardware_profile.as_str() {
            "rx580_vulkan" => "vulkan".into(),
            "tesla_t4" => "cuda".into(),
            _ => "cpu".into(),
        }
    }
}

fn extract_prompt(value: &serde_json::Value) -> String {
    if let Some(messages) = value["messages"].as_array() {
        let mut parts = Vec::new();
        for m in messages {
            let role = m["role"].as_str().unwrap_or("user");
            let content = m["content"].as_str().unwrap_or("");
            if role == "system" {
                parts.push(format!("System: {content}"));
            } else if role == "user" {
                parts.push(content.to_string());
            }
        }
        return parts.join("\n");
    }
    value["prompt"].as_str().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// Leaderboard / catalog payloads
// ---------------------------------------------------------------------------

fn default_profiles() -> serde_json::Value {
    json!({
        "rx580_vulkan": {"label": "AMD Radeon RX 580 8GB (Vulkan)", "backend": "vulkan", "vram_mb": 8192, "ram_mb": 16384},
        "nothing_phone_2a": {"label": "Nothing Phone 2a 12GB (CPU NEON)", "backend": "cpu", "vram_mb": 0, "ram_mb": 12288},
        "i5_4690_ddr3": {"label": "Intel i5-4690 16GB DDR3 (CPU AVX2)", "backend": "cpu", "vram_mb": 0, "ram_mb": 16384},
        "tesla_t4": {"label": "NVIDIA Tesla T4 16GB (CUDA)", "backend": "cuda", "vram_mb": 16384, "ram_mb": 30720},
    })
}

fn load_catalog_file(state: &AppState) -> Option<catalog::Catalog> {
    let path = state
        .config
        .catalog_path
        .clone()
        .or_else(catalog::find_catalog_file)?;
    catalog::Catalog::load(&path).ok()
}

fn leaderboard_payload(state: &AppState, profile: &str) -> anyhow::Result<serde_json::Value> {
    let db = state.config.db.lock().unwrap();
    let mut entries = db.leaderboard(profile)?;
    drop(db);
    let mask = catalog::pareto_frontier(&entries);
    for (entry, optimal) in entries.iter_mut().zip(mask) {
        entry.pareto_optimal = Some(optimal);
    }
    let profiles = load_catalog_file(state)
        .map(|c| serde_json::to_value(&c.hardware_profiles).unwrap_or_else(|_| default_profiles()))
        .unwrap_or_else(default_profiles);
    let labels: serde_json::Map<String, serde_json::Value> = catalog::PILLARS
        .iter()
        .map(|p| ((*p).to_string(), json!(catalog::pillar_label(p))))
        .collect();
    Ok(json!({
        "ok": true,
        "pillars": catalog::PILLARS,
        "pillar_labels": labels,
        "profiles": profiles,
        "entries": entries,
    }))
}

fn catalog_payload(state: &AppState) -> anyhow::Result<serde_json::Value> {
    if let Some(catalog) = load_catalog_file(state) {
        return Ok(serde_json::to_value(&catalog)?);
    }
    let db = state.config.db.lock().unwrap();
    let entries = db.leaderboard("")?;
    let labels: serde_json::Map<String, serde_json::Value> = catalog::PILLARS
        .iter()
        .map(|p| ((*p).to_string(), json!(catalog::pillar_label(p))))
        .collect();
    Ok(json!({
        "schema_version": 1,
        "generated": util::iso8601_now(),
        "provenance_note": "Entries stored in the local Vibe Local database.",
        "pillars": catalog::PILLARS,
        "pillar_labels": labels,
        "hardware_profiles": default_profiles(),
        "entries": entries,
    }))
}

// ---------------------------------------------------------------------------
// Static files (web leaderboard) with traversal protection + CSP
// ---------------------------------------------------------------------------

fn find_web_root(state: &AppState) -> Option<PathBuf> {
    if let Some(root) = &state.config.web_root
        && root.is_dir()
    {
        return Some(root.clone());
    }
    let mut bases = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        bases.push(dir.to_path_buf());
        bases.push(dir.join(".."));
        bases.push(dir.join("../.."));
    }
    if let Ok(cwd) = std::env::current_dir() {
        bases.push(cwd);
    }
    for base in bases {
        let candidate = base.join("leaderboard-web");
        if candidate.join("index.html").is_file() {
            return Some(candidate);
        }
    }
    None
}

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn serve_static(state: &AppState, rel: &str) -> HttpResult {
    if rel.contains("..") || rel.contains('\\') {
        return Ok(err_json(400, "invalid path"));
    }
    let Some(root) = find_web_root(state) else {
        return Ok(err_json(
            503,
            "leaderboard-web assets not found; run from the repository root or set VIBE_WEB_ROOT",
        ));
    };
    let full = root.join(rel);
    let Ok(canonical) = full.canonicalize() else {
        return Ok(err_json(404, "not found"));
    };
    let Ok(root_canonical) = root.canonicalize() else {
        return Ok(err_json(500, "web root unavailable"));
    };
    if !canonical.starts_with(&root_canonical) || !canonical.is_file() {
        return Ok(err_json(404, "not found"));
    }
    let bytes = std::fs::read(&canonical)?;
    let resp = Response::from_data(bytes)
        .with_status_code(200)
        .with_header(header("Content-Type", content_type_for(rel)))
        .with_header(header("Content-Security-Policy", CSP))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header("Cache-Control", "no-cache"));
    Ok(resp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vibe_local_core::Db;
    use vibe_local_engine::EngineConfig;

    fn test_state(lan: bool, token: Option<&str>) -> (Arc<ServerHandle>, String) {
        let db = Db::open_in_memory().unwrap().shared();
        let engine = Engine::start(EngineConfig {
            internal_port: 20198,
            ..Default::default()
        });
        let config = ServerConfig {
            port: 0,
            lan,
            token: token.map(|t| t.to_string()),
            db,
            engine,
            web_root: None,
            catalog_path: None,
        };
        let handle = start_server(config).unwrap();
        let base = format!("http://127.0.0.1:{}", handle.port);
        (handle, base)
    }

    fn get(url: &str) -> ureq::Response {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build()
            .get(url)
            .call()
            .unwrap()
    }

    #[test]
    fn hardware_and_recommendations() {
        let (_h, base) = test_state(false, None);
        let hw: serde_json::Value = get(&format!("{base}/api/hardware")).into_json().unwrap();
        assert!(hw["ram_total_mb"].as_u64().unwrap() > 0);
        let rec: serde_json::Value = get(&format!("{base}/api/recommendations"))
            .into_json()
            .unwrap();
        assert!(rec["tier"].as_str().is_some());
        assert!(rec["candidates"].as_array().is_some());
    }

    #[test]
    fn lan_auth_enforced() {
        let (_h, base) = test_state(true, Some("secret-token-123"));
        // No token -> 401
        let err = ureq::get(&format!("{base}/api/hardware"))
            .call()
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(401, _)));
        // Wrong token -> 401
        let err = ureq::get(&format!("{base}/api/hardware"))
            .set("X-Vibe-Local-Token", "wrong")
            .call()
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(401, _)));
        // Correct header -> 200
        let resp = ureq::get(&format!("{base}/api/hardware"))
            .set("X-Vibe-Local-Token", "secret-token-123")
            .call()
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Bearer form -> 200
        let resp = ureq::get(&format!("{base}/api/status"))
            .set("Authorization", "Bearer secret-token-123")
            .call()
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Static UI stays public
        let resp = ureq::get(&format!("{base}/")).call();
        assert!(resp.is_ok() || matches!(resp, Err(ureq::Error::Status(503, _))));
    }

    #[test]
    fn lan_without_token_rejects_all_api() {
        let (_h, base) = test_state(true, None);
        let err = ureq::get(&format!("{base}/api/hardware"))
            .call()
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(401, _)));
    }

    #[test]
    fn csrf_content_type_guard() {
        let (_h, base) = test_state(true, Some("t"));
        // POST without JSON content-type -> 415
        let err = ureq::post(&format!("{base}/api/unload"))
            .set("X-Vibe-Local-Token", "t")
            .send_string("")
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(415, _)));
        // With JSON content-type -> 200
        let resp = ureq::post(&format!("{base}/api/unload"))
            .set("X-Vibe-Local-Token", "t")
            .set("Content-Type", "application/json")
            .send_string("{}")
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Admin header alternative
        let resp = ureq::post(&format!("{base}/api/unload"))
            .set("X-Vibe-Local-Token", "t")
            .set("X-Vibe-Admin", "1")
            .send_string("")
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[test]
    fn leaderboard_submit_roundtrip() {
        let (_h, base) = test_state(false, None);
        let entry = json!({
            "model_id": "TestModel-7B",
            "hf_repo": "bartowski/TestModel-7B-GGUF",
            "filename": "TestModel-7B-Q4_K_M.gguf",
            "download_url": "https://huggingface.co/bartowski/TestModel-7B-GGUF/resolve/main/TestModel-7B-Q4_K_M.gguf",
            "quant": "Q4_K_M",
            "architecture": "qwen2",
            "family": "dense",
            "total_params_b": 7.6,
            "active_params_b": 7.6,
            "size_bytes": 4680000000u64,
            "context_length": 32768,
            "hardware_profile": "rx580_vulkan",
            "provenance": "measured",
            "pillars": {
                "code_generation": {"score": 0.8, "metric": "pass@1", "n": 164},
                "math_reasoning": {"score": 0.7, "metric": "accuracy", "n": 50}
            },
            "throughput": {"prefill_tps": 400.0, "decode_tps": 30.0, "ttft_ms": 150.0, "peak_ram_mb": 5200.0}
        });
        let resp = ureq::post(&format!("{base}/api/leaderboard/submit"))
            .send_json(json!({"entry": entry}))
            .unwrap();
        assert_eq!(resp.status(), 200);
        let board: serde_json::Value = get(&format!("{base}/api/leaderboard?profile=rx580_vulkan"))
            .into_json()
            .unwrap();
        let entries = board["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["provenance"], "measured");
        assert!(entries[0]["vibe_quality_score"].as_f64().is_some());
        assert!(entries[0]["pareto_optimal"].as_bool().is_some());
        assert_eq!(board["pillars"].as_array().unwrap().len(), 10);
        // seed_estimate submission rejected
        let mut seed = entry.clone();
        seed["provenance"] = json!("seed_estimate");
        let err = ureq::post(&format!("{base}/api/leaderboard/submit"))
            .send_json(seed)
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(400, _)));
        // invalid entry rejected
        let mut bad = entry.clone();
        bad["family"] = json!("alien");
        let err = ureq::post(&format!("{base}/api/leaderboard/submit"))
            .send_json(bad)
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Status(500, _)));
    }

    #[test]
    fn chat_headers_and_think_strip() {
        let (handle, base) = test_state(false, None);
        if vibe_local_engine::supervisor::find_llama_server(None).is_some() {
            handle.shutdown();
            return; // real llama-server present; builtin path not exercised
        }
        // Load a synthetic GGUF (builtin micro-eval mode).
        let dir = tempfile::tempdir().unwrap();
        let model_path = dir.path().join("tiny-Q4_K_M.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&vibe_local_core::gguf::GGUF_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        std::fs::write(&model_path, bytes).unwrap();
        ureq::post(&format!("{base}/api/load"))
            .send_json(json!({"path": model_path.to_string_lossy(), "backend": "cpu"}))
            .unwrap();

        let resp = ureq::post(&format!("{base}/v1/chat/completions"))
            .send_json(json!({"messages": [{"role": "user", "content": "What is 6 * 7?"}], "strip_think": true}))
            .unwrap();
        assert_eq!(resp.header("X-Vibe-Router-Cost-Micros"), Some("0"));
        assert!(resp.header("X-Vibe-Local-Tok-Per-Sec").is_some());
        assert_eq!(resp.header("X-Vibe-Local-Backend"), Some("cpu"));
        let body: serde_json::Value = resp.into_json().unwrap();
        assert!(
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("42")
        );

        let models: serde_json::Value = get(&format!("{base}/v1/models")).into_json().unwrap();
        assert_eq!(models["data"].as_array().unwrap().len(), 1);
        handle.shutdown();
    }

    #[test]
    fn csp_on_api_responses() {
        let (_h, base) = test_state(false, None);
        let resp = get(&format!("{base}/api/status"));
        let csp = resp.header("Content-Security-Policy").unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn url_param_parsing() {
        assert_eq!(
            query_param("/api/leaderboard?profile=rx580_vulkan", "profile").as_deref(),
            Some("rx580_vulkan")
        );
        assert_eq!(
            query_param("/api/discover?query=qwen%20coder", "query").as_deref(),
            Some("qwen coder")
        );
        assert_eq!(query_param("/api/x", "y"), None);
    }
}
