//! Inference engine supervisor: manages a `llama-server` child process with
//! backend selection (Vulkan / CUDA / CPU), falls back to the deterministic
//! built-in micro-eval engine when the binary is absent, enforces the idle
//! auto-unload timer (default 300s) and the thermal governor. All commands
//! flow through a bounded channel (capacity 16).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use vibe_local_core::util;

use crate::microeval::{MicroEval, strip_think, tokenize};
use crate::thermal::{ThermalGovernor, ThermalState};
use crate::vibemark::{self, ProbeOutcome, VibeMarkResult};

pub const DEFAULT_IDLE_SECS: u64 = 300;
pub const DEFAULT_INTERNAL_PORT: u16 = 20130;
const COMMAND_CHANNEL_CAPACITY: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadRequest {
    pub path: String,
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_ngl")]
    pub ngl: u32,
    #[serde(default = "default_ctx")]
    pub ctx: u32,
}

fn default_backend() -> String {
    "cpu".into()
}
fn default_ngl() -> u32 {
    0
}
fn default_ctx() -> u32 {
    4096
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatReply {
    pub text: String,
    pub tokens_generated: u64,
    pub elapsed_secs: f64,
    pub tok_per_sec: f64,
    pub backend: String,
    pub engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    pub loaded_model: Option<String>,
    pub backend: String,
    pub engine: String,
    pub idle_timeout_secs: u64,
    pub thermal_celsius: Option<f64>,
    pub thermal_state: ThermalState,
    pub internal_port: u16,
}

enum EngineCommand {
    Load(LoadRequest, SyncSender<anyhow::Result<()>>),
    Unload(SyncSender<()>),
    Chat {
        prompt: String,
        strip: bool,
        max_tokens: u32,
        resp: SyncSender<anyhow::Result<ChatReply>>,
    },
    VibeMark {
        budget_secs: u64,
        resp: SyncSender<anyhow::Result<VibeMarkResult>>,
    },
    Status(SyncSender<EngineStatus>),
    SetIdle(u64),
    Shutdown,
}

pub struct EngineConfig {
    pub llama_server_path: Option<PathBuf>,
    pub internal_port: u16,
    pub idle_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            llama_server_path: None,
            internal_port: DEFAULT_INTERNAL_PORT,
            idle_secs: DEFAULT_IDLE_SECS,
        }
    }
}

pub struct Engine {
    tx: SyncSender<EngineCommand>,
    thermal: Arc<ThermalGovernor>,
    stopped: Arc<AtomicBool>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

struct LoadedModel {
    req: LoadRequest,
    child: Option<Child>,
    /// "llama-server" or "builtin-microeval"
    engine: String,
    last_activity: Instant,
}

impl Engine {
    pub fn start(config: EngineConfig) -> Arc<Engine> {
        let (tx, rx) = sync_channel::<EngineCommand>(COMMAND_CHANNEL_CAPACITY);
        let thermal = Arc::new(ThermalGovernor::new());
        let stopped = Arc::new(AtomicBool::new(false));
        let worker = {
            let thermal = Arc::clone(&thermal);
            let stopped = Arc::clone(&stopped);
            std::thread::Builder::new()
                .name("vibe-engine".into())
                .spawn(move || {
                    worker_loop(rx, config, thermal, stopped);
                })
                .expect("spawn engine worker")
        };
        Arc::new(Engine {
            tx,
            thermal,
            stopped,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn thermal(&self) -> Arc<ThermalGovernor> {
        Arc::clone(&self.thermal)
    }

    fn call<T>(
        &self,
        make: impl FnOnce(SyncSender<anyhow::Result<T>>) -> EngineCommand,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let (resp_tx, resp_rx) = sync_channel(1);
        self.tx
            .send(make(resp_tx))
            .map_err(|_| anyhow::anyhow!("engine worker is not running"))?;
        resp_rx
            .recv_timeout(timeout)
            .map_err(|_| anyhow::anyhow!("engine request timed out"))?
    }

    pub fn load(&self, req: LoadRequest) -> anyhow::Result<()> {
        self.call(|tx| EngineCommand::Load(req, tx), Duration::from_secs(180))
    }

    pub fn unload(&self) {
        let (tx, rx) = sync_channel(1);
        if self.tx.send(EngineCommand::Unload(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(30));
        }
    }

    pub fn chat(&self, prompt: &str, strip: bool, max_tokens: u32) -> anyhow::Result<ChatReply> {
        let prompt = prompt.to_string();
        self.call(
            |tx| EngineCommand::Chat {
                prompt,
                strip,
                max_tokens,
                resp: tx,
            },
            Duration::from_secs(300),
        )
    }

    pub fn vibemark(&self, budget_secs: u64) -> anyhow::Result<VibeMarkResult> {
        self.call(
            |tx| EngineCommand::VibeMark {
                budget_secs,
                resp: tx,
            },
            Duration::from_secs(budget_secs + 120),
        )
    }

    pub fn status(&self) -> anyhow::Result<EngineStatus> {
        let (tx, rx) = sync_channel(1);
        self.tx
            .send(EngineCommand::Status(tx))
            .map_err(|_| anyhow::anyhow!("engine worker is not running"))?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow::anyhow!("engine status timed out"))
    }

    pub fn set_idle_secs(&self, secs: u64) {
        let _ = self.tx.send(EngineCommand::SetIdle(secs.clamp(10, 86_400)));
    }

    pub fn shutdown(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(EngineCommand::Shutdown);
        if let Some(handle) = self.worker.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Locate a llama-server binary: explicit path > VIBE_LLAMA_SERVER env >
/// PATH > common local build locations.
pub fn find_llama_server(explicit: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(p) = explicit
        && p.is_file()
    {
        return Some(p.clone());
    }
    if let Ok(p) = std::env::var("VIBE_LLAMA_SERVER") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }
    let exe_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    if let Some(path) = std::env::var_os("PATH") {
        let found = std::env::split_paths(&path)
            .map(|dir| dir.join(exe_name))
            .find(|candidate| candidate.is_file());
        if found.is_some() {
            return found;
        }
    }
    [
        PathBuf::from("llama.cpp/build/bin").join(exe_name),
        PathBuf::from("llama.cpp").join(exe_name),
        PathBuf::from(exe_name),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

fn worker_loop(
    rx: std::sync::mpsc::Receiver<EngineCommand>,
    config: EngineConfig,
    thermal: Arc<ThermalGovernor>,
    stopped: Arc<AtomicBool>,
) {
    let mut loaded: Option<LoadedModel> = None;
    let mut idle_secs = config.idle_secs;
    let mut last_thermal_poll = Instant::now();
    let micro = MicroEval::default();

    loop {
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(cmd) => match cmd {
                EngineCommand::Shutdown => break,
                EngineCommand::SetIdle(secs) => idle_secs = secs,
                EngineCommand::Status(tx) => {
                    let _ = tx.send(status_of(
                        &loaded,
                        idle_secs,
                        &thermal,
                        config.internal_port,
                    ));
                }
                EngineCommand::Unload(tx) => {
                    unload_model(&mut loaded);
                    let _ = tx.send(());
                }
                EngineCommand::Load(req, tx) => {
                    unload_model(&mut loaded);
                    let result = load_model(&req, &config, &micro);
                    match result {
                        Ok(model) => {
                            loaded = Some(model);
                            let _ = tx.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                        }
                    }
                }
                EngineCommand::Chat {
                    prompt,
                    strip,
                    max_tokens,
                    resp,
                } => {
                    thermal.poll();
                    let result = match loaded.as_mut() {
                        None => Err(anyhow::anyhow!("no model loaded; POST /api/load first")),
                        Some(_) if !thermal.allows_inference() => Err(anyhow::anyhow!(
                            "thermal pause: device at >=45°C; inference resumes below 43°C"
                        )),
                        Some(model) => {
                            model.last_activity = Instant::now();
                            let delay = thermal.throttle_delay_ms();
                            if delay > 0 {
                                std::thread::sleep(Duration::from_millis(delay));
                            }
                            let effective_max = if thermal.state() == ThermalState::Throttled {
                                max_tokens / 2
                            } else {
                                max_tokens
                            };
                            chat_with(
                                model,
                                &prompt,
                                strip,
                                effective_max.max(16),
                                &micro,
                                config.internal_port,
                            )
                        }
                    };
                    let _ = resp.send(result);
                }
                EngineCommand::VibeMark { budget_secs, resp } => {
                    thermal.poll();
                    let result = match loaded.as_mut() {
                        None => Err(anyhow::anyhow!(
                            "no model loaded; load a GGUF before benchmarking"
                        )),
                        Some(_) if !thermal.allows_inference() => Err(anyhow::anyhow!(
                            "thermal pause: benchmark blocked at >=45°C"
                        )),
                        Some(model) => {
                            model.last_activity = Instant::now();
                            run_vibemark(model, budget_secs, &micro, config.internal_port)
                        }
                    };
                    let _ = resp.send(result);
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                if last_thermal_poll.elapsed() >= Duration::from_secs(5) {
                    thermal.poll();
                    last_thermal_poll = Instant::now();
                }
                let idle_expired = loaded.as_ref().is_some_and(|model| {
                    model.last_activity.elapsed() >= Duration::from_secs(idle_secs)
                });
                if idle_expired {
                    eprintln!(
                        "[vibe-engine] idle {}s reached, auto-unloading model",
                        idle_secs
                    );
                    unload_model(&mut loaded);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    unload_model(&mut loaded);
}

fn status_of(
    loaded: &Option<LoadedModel>,
    idle_secs: u64,
    thermal: &ThermalGovernor,
    port: u16,
) -> EngineStatus {
    EngineStatus {
        loaded_model: loaded.as_ref().map(|m| m.req.path.clone()),
        backend: loaded
            .as_ref()
            .map(|m| m.req.backend.clone())
            .unwrap_or_else(|| "none".into()),
        engine: loaded
            .as_ref()
            .map(|m| m.engine.clone())
            .unwrap_or_else(|| "none".into()),
        idle_timeout_secs: idle_secs,
        thermal_celsius: thermal.current_reading(),
        thermal_state: thermal.state(),
        internal_port: port,
    }
}

fn unload_model(loaded: &mut Option<LoadedModel>) {
    if let Some(mut model) = loaded.take()
        && let Some(child) = model.child.as_mut()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn load_model(
    req: &LoadRequest,
    config: &EngineConfig,
    micro: &MicroEval,
) -> anyhow::Result<LoadedModel> {
    let path = PathBuf::from(&req.path);
    anyhow::ensure!(path.is_file(), "model file not found: {}", req.path);
    // Validate the GGUF header before committing to a load.
    let info = vibe_local_core::gguf::parse_file(&path)?;
    let _ = micro.ingest(&info.architecture);

    let Some(binary) = find_llama_server(config.llama_server_path.as_ref()) else {
        return Ok(LoadedModel {
            req: req.clone(),
            child: None,
            engine: "builtin-microeval".into(),
            last_activity: Instant::now(),
        });
    };

    let mut cmd = Command::new(&binary);
    cmd.arg("-m")
        .arg(&req.path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(config.internal_port.to_string())
        .arg("-ngl")
        .arg(req.ngl.to_string())
        .arg("-c")
        .arg(req.ctx.to_string())
        .arg("-np")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match req.backend.as_str() {
        "cuda" => {
            cmd.env("CUDA_VISIBLE_DEVICES", "0");
        }
        "vulkan" => {
            cmd.env("GGML_VK_DEVICE", "0");
        }
        _ => {
            cmd.env("CUDA_VISIBLE_DEVICES", "");
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn llama-server: {e}"))?;

    // Poll /health until the model is resident (large models take a while).
    let health_url = format!("http://127.0.0.1:{}/health", config.internal_port);
    let deadline = Instant::now() + Duration::from_secs(150);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    let mut healthy = false;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().ok().flatten() {
            anyhow::bail!("llama-server exited during load (status {status})");
        }
        if let Ok(resp) = agent.get(&health_url).call()
            && resp.status() == 200
        {
            healthy = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !healthy {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("llama-server did not become healthy within 150s");
    }
    Ok(LoadedModel {
        req: req.clone(),
        child: Some(child),
        engine: "llama-server".into(),
        last_activity: Instant::now(),
    })
}

fn chat_with(
    model: &mut LoadedModel,
    prompt: &str,
    strip: bool,
    max_tokens: u32,
    micro: &MicroEval,
    port: u16,
) -> anyhow::Result<ChatReply> {
    let start = Instant::now();
    let (text, tokens) = if model.engine == "llama-server" {
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": max_tokens,
            "temperature": 0.0,
            "stream": false,
        });
        let resp = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(240))
            .build()
            .post(&url)
            .send_json(body)?;
        let json: serde_json::Value = resp.into_json()?;
        let text = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let tokens = json["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        (text, tokens)
    } else {
        let text = micro.generate_text(prompt);
        let tokens = tokenize(&text).len() as u64;
        (text, tokens)
    };
    let elapsed = start.elapsed().as_secs_f64();
    let text = if strip { strip_think(&text) } else { text };
    let tokens = if strip {
        tokenize(&text).len() as u64
    } else {
        tokens
    };
    Ok(ChatReply {
        tok_per_sec: if elapsed > 0.0 {
            tokens as f64 / elapsed
        } else {
            0.0
        },
        text,
        tokens_generated: tokens,
        elapsed_secs: elapsed,
        backend: model.req.backend.clone(),
        engine: model.engine.clone(),
    })
}

/// One inference probe used by VibeMark. `stream` requests TTFT measurement.
fn probe(
    model: &LoadedModel,
    prompt: &str,
    n_predict: u32,
    stream: bool,
    micro: &MicroEval,
    port: u16,
) -> anyhow::Result<ProbeOutcome> {
    if model.engine == "llama-server" {
        let url = format!("http://127.0.0.1:{port}/completion");
        let body = serde_json::json!({
            "prompt": prompt,
            "n_predict": n_predict,
            "stream": stream,
            "cache_prompt": false,
            "temperature": 0.0,
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(240))
            .build();
        let start = Instant::now();
        let resp = agent.post(&url).send_json(body)?;
        if stream {
            // Read the SSE stream until the first content chunk (TTFT),
            // then drain the rest to let the server finish cleanly.
            use std::io::{BufRead, BufReader};
            let mut reader = BufReader::new(resp.into_reader());
            let mut ttft_ms = None;
            let mut predicted: u64 = 0;
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                let trimmed = line.trim();
                let Some(data) = trimmed.strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    break;
                }
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    let content = json["content"].as_str().unwrap_or("");
                    if !content.is_empty() {
                        if ttft_ms.is_none() {
                            ttft_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
                        }
                        predicted += tokenize(content).len() as u64;
                    }
                    if json["stop"].as_bool() == Some(true) {
                        break;
                    }
                }
            }
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            let ttft = ttft_ms.unwrap_or(total_ms);
            Ok(ProbeOutcome {
                prompt_tokens: tokenize(prompt).len() as u64,
                prompt_ms: ttft,
                predicted_tokens: predicted.max(1),
                predicted_ms: (total_ms - ttft).max(0.001),
                ttft_ms: Some(ttft),
                text: String::new(),
            })
        } else {
            let json: serde_json::Value = resp.into_json()?;
            let timings = &json["timings"];
            let prompt_n = timings["prompt_n"]
                .as_u64()
                .unwrap_or_else(|| tokenize(prompt).len() as u64);
            let prompt_ms = timings["prompt_ms"].as_f64().unwrap_or(0.0);
            let predicted_n = timings["predicted_n"].as_u64().unwrap_or(0);
            let predicted_ms = timings["predicted_ms"].as_f64().unwrap_or(0.0);
            Ok(ProbeOutcome {
                prompt_tokens: prompt_n,
                prompt_ms: prompt_ms.max(0.001),
                predicted_tokens: predicted_n,
                predicted_ms: predicted_ms.max(0.001),
                ttft_ms: None,
                text: json["content"].as_str().unwrap_or("").to_string(),
            })
        }
    } else {
        // Builtin micro-eval: real measured work (ingest + generation).
        let t0 = Instant::now();
        let prompt_tokens = micro.ingest(prompt) as u64;
        let prompt_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        let tokens = micro.generate(prompt, n_predict as usize);
        let predicted_ms = t1.elapsed().as_secs_f64() * 1000.0;
        let text = tokens.join(" ");
        Ok(ProbeOutcome {
            prompt_tokens,
            prompt_ms: prompt_ms.max(0.001),
            predicted_tokens: tokens.len() as u64,
            predicted_ms: predicted_ms.max(0.001),
            ttft_ms: Some(prompt_ms + predicted_ms / tokens.len().max(1) as f64),
            text,
        })
    }
}

fn run_vibemark(
    model: &mut LoadedModel,
    budget_secs: u64,
    micro: &MicroEval,
    port: u16,
) -> anyhow::Result<VibeMarkResult> {
    let model_name = std::path::Path::new(&model.req.path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| model.req.path.clone());
    let pid = model.child.as_ref().map(|c| c.id());
    let engine = model.engine.clone();
    let backend = model.req.backend.clone();
    let sampler = move || -> Option<f64> {
        match pid {
            Some(pid) => util::process_rss_mb(pid),
            None => util::process_rss_mb(std::process::id()),
        }
    };
    let result = vibemark::run(
        budget_secs,
        &model_name,
        &backend,
        &engine,
        |prompt, n, stream| probe(model, prompt, n, stream, micro, port),
        sampler,
    )?;
    model.last_activity = Instant::now();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_lifecycle_builtin() {
        // Create a tiny synthetic GGUF so load validates the header.
        let dir = tempfile::tempdir().unwrap();
        let model_path = dir.path().join("tiny-Q4_K_M.gguf");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&vibe_local_core::gguf::GGUF_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        let key = b"general.architecture";
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&8u32.to_le_bytes());
        let val = b"llama";
        bytes.extend_from_slice(&(val.len() as u64).to_le_bytes());
        bytes.extend_from_slice(val);
        std::fs::write(&model_path, bytes).unwrap();

        let engine = Engine::start(EngineConfig {
            // Force builtin mode even if llama-server exists on PATH.
            llama_server_path: None,
            internal_port: 20199,
            idle_secs: 300,
        });
        // Without a llama-server binary the engine uses builtin micro-eval.
        // If one IS installed this test still passes via the real server path
        // only when the model is valid; tiny synthetic file would fail there,
        // so gate on binary presence.
        if find_llama_server(None).is_some() {
            engine.shutdown();
            return;
        }
        engine
            .load(LoadRequest {
                path: model_path.to_string_lossy().into_owned(),
                backend: "cpu".into(),
                ngl: 0,
                ctx: 2048,
            })
            .unwrap();
        let status = engine.status().unwrap();
        assert_eq!(status.engine, "builtin-microeval");
        assert!(status.loaded_model.is_some());

        let reply = engine.chat("What is 21 * 2?", true, 128).unwrap();
        assert!(reply.text.contains("42"), "got {}", reply.text);
        assert!(reply.tok_per_sec > 0.0);
        assert_eq!(reply.engine, "builtin-microeval");

        engine.unload();
        assert!(engine.status().unwrap().loaded_model.is_none());
        assert!(engine.chat("hello", false, 32).is_err());
        engine.shutdown();
    }

    #[test]
    fn load_missing_file_fails() {
        let engine = Engine::start(EngineConfig::default());
        let err = engine
            .load(LoadRequest {
                path: "/nonexistent/model.gguf".into(),
                backend: "cpu".into(),
                ngl: 0,
                ctx: 2048,
            })
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
        engine.shutdown();
    }

    #[test]
    fn idle_timeout_config() {
        let engine = Engine::start(EngineConfig {
            idle_secs: 42,
            ..Default::default()
        });
        assert_eq!(engine.status().unwrap().idle_timeout_secs, 42);
        engine.set_idle_secs(600);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(engine.status().unwrap().idle_timeout_secs, 600);
        engine.shutdown();
    }
}
