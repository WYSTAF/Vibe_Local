//! # vibe-local-jni
//!
//! Android JNI bridge (`Java_com_vibelocal_app_core_VibeLocalBridge_*`) and
//! plain C-ABI wrappers exposing the full Vibe Local stack to Kotlin:
//! hardware scan, MoE-vs-Dense recommendations, GGUF inspection, VibeMark,
//! leaderboard queries and Vibe-Link server control. Every function returns
//! a JSON string `{"ok":true,"data":...}` or `{"ok":false,"error":"..."}`.
//!
//! Licensed under Apache-2.0.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value, json};

use vibe_local_cli::{commands, discovery, server};
use vibe_local_core::{Db, SharedDb};
use vibe_local_engine::Engine;

// ---------------------------------------------------------------------------
// Shared bridge state
// ---------------------------------------------------------------------------

struct BridgeState {
    db: SharedDb,
    engine: Arc<Engine>,
    server: Option<Arc<server::ServerHandle>>,
    discovery: Option<discovery::DiscoveryHandle>,
    data_dir: PathBuf,
    lan: bool,
}

fn state_cell() -> &'static Mutex<Option<BridgeState>> {
    static CELL: OnceLock<Mutex<Option<BridgeState>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

fn default_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VIBE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let local = PathBuf::from("vibe_data");
    if std::fs::create_dir_all(&local).is_ok() {
        return local;
    }
    std::env::temp_dir().join("vibe-local")
}

fn with_state<T>(
    data_dir: Option<String>,
    f: impl FnOnce(&mut BridgeState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut guard = state_cell().lock().unwrap();
    if guard.is_none() {
        let dir = data_dir.map(PathBuf::from).unwrap_or_else(default_data_dir);
        std::fs::create_dir_all(&dir)?;
        let db = Db::open(&dir.join("vibe_local.db"))?.shared();
        let engine = Engine::start(commands::engine_config(300));
        *guard = Some(BridgeState {
            db,
            engine,
            server: None,
            discovery: None,
            data_dir: dir,
            lan: false,
        });
    }
    f(guard.as_mut().unwrap())
}

fn wrap(result: anyhow::Result<Value>) -> Value {
    match result {
        Ok(v) => json!({"ok": true, "data": v}),
        Err(e) => json!({"ok": false, "error": format!("{e:#}")}),
    }
}

// ---------------------------------------------------------------------------
// API implementations (shared by JNI and C-ABI)
// ---------------------------------------------------------------------------

fn api_scan(data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| commands::scan(&s.db)))
}

fn api_recommend(data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| {
        let report = vibe_local_core::HardwareProbe::scan();
        s.db.lock().unwrap().insert_scan(&report)?;
        Ok(serde_json::to_value(vibe_local_core::recommend(&report))?)
    }))
}

fn api_inspect(path: &str, data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| {
        let info = commands::inspect(&s.db, std::path::Path::new(path))?;
        Ok(serde_json::to_value(info)?)
    }))
}

fn api_vibemark(path: &str, budget_secs: u64, data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| {
        let models_dir = s.data_dir.join("models");
        let outcome = commands::bench(&s.db, &s.engine, path, &models_dir, budget_secs)?;
        Ok(json!({"vibemark": outcome.result, "entry": outcome.entry}))
    }))
}

fn api_leaderboard(profile: &str, data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| {
        commands::import_seed_catalog(&s.db)?;
        commands::leaderboard_json(&s.db, profile)
    }))
}

fn api_submit(entry_json: &str, data_dir: Option<String>) -> Value {
    wrap(with_state(data_dir, |s| {
        let mut entry: vibe_local_core::CatalogEntry = serde_json::from_str(entry_json)?;
        if entry.provenance != "measured" {
            entry.provenance = "submitted".into();
        }
        entry.validate()?;
        s.db.lock().unwrap().upsert_catalog_entry(&entry)?;
        Ok(json!({"model_id": entry.model_id}))
    }))
}

fn api_start_server(port: u16, lan: bool, token: &str, data_dir: &str) -> Value {
    wrap(with_state(Some(data_dir.to_string()), |s| {
        if let Some(old) = s.server.take() {
            old.shutdown();
        }
        if let Some(old) = s.discovery.take() {
            old.shutdown();
        }
        commands::import_seed_catalog(&s.db)?;
        let token = if lan {
            let explicit = if token.is_empty() {
                None
            } else {
                Some(token.to_string())
            };
            Some(commands::resolve_lan_token(&s.db, explicit)?)
        } else if token.is_empty() {
            None
        } else {
            Some(token.to_string())
        };
        let config = server::ServerConfig {
            port,
            lan,
            token: token.clone(),
            db: s.db.clone(),
            engine: s.engine.clone(),
            web_root: std::env::var("VIBE_WEB_ROOT").ok().map(PathBuf::from),
            catalog_path: None,
        };
        let handle = server::start_server(config)?;
        let disc = discovery::start_discovery(discovery::DiscoveryConfig {
            http_port: handle.port,
            lan,
            udp_port: discovery::UDP_PROBE_PORT,
            mdns_port: discovery::MDNS_PORT,
        })
        .ok();
        let actual_port = handle.port;
        s.server = Some(handle);
        s.discovery = disc;
        s.lan = lan;
        Ok(json!({"port": actual_port, "lan": lan, "token": token}))
    }))
}

fn api_stop_server() -> Value {
    wrap(with_state(None, |s| {
        if let Some(handle) = s.server.take() {
            handle.shutdown();
        }
        if let Some(disc) = s.discovery.take() {
            disc.shutdown();
        }
        s.engine.unload();
        s.lan = false;
        Ok(json!({"stopped": true}))
    }))
}

fn api_status() -> Value {
    wrap(with_state(None, |s| {
        let status = s.engine.status()?;
        Ok(json!({
            "loaded_model": status.loaded_model,
            "backend": status.backend,
            "engine": status.engine,
            "idle_timeout_secs": status.idle_timeout_secs,
            "thermal": {"celsius": status.thermal_celsius, "state": status.thermal_state},
            "server_running": s.server.is_some(),
            "port": s.server.as_ref().map(|h| h.port),
            "lan": s.lan,
            "data_dir": s.data_dir.to_string_lossy(),
        }))
    }))
}

fn api_chat(prompt: &str, strip_think: bool) -> Value {
    wrap(with_state(None, |s| {
        let reply = s.engine.chat(prompt, strip_think, 1024)?;
        Ok(serde_json::to_value(reply)?)
    }))
}

fn api_unload() -> Value {
    wrap(with_state(None, |s| {
        s.engine.unload();
        Ok(json!({"unloaded": true}))
    }))
}

fn api_set_thermal(celsius: f64) -> Value {
    wrap(with_state(None, |s| {
        anyhow::ensure!(
            (-40.0..=125.0).contains(&celsius),
            "implausible temperature {celsius}"
        );
        s.engine.thermal().set_override(celsius);
        Ok(json!({"celsius": celsius}))
    }))
}

fn api_set_idle(seconds: u64) -> Value {
    wrap(with_state(None, |s| {
        s.engine.set_idle_secs(seconds);
        Ok(json!({"idle_timeout_secs": seconds}))
    }))
}

fn api_set_data_dir(data_dir: &str) -> Value {
    wrap((|| {
        let mut guard = state_cell().lock().unwrap();
        if let Some(s) = guard.as_mut() {
            if s.server.is_some() {
                anyhow::bail!("stop the server before changing the data directory");
            }
            let dir = PathBuf::from(data_dir);
            std::fs::create_dir_all(&dir)?;
            s.db = Db::open(&dir.join("vibe_local.db"))?.shared();
            s.data_dir = dir;
        }
        Ok(json!({"data_dir": data_dir}))
    })())
}

// ---------------------------------------------------------------------------
// JNI exports
// ---------------------------------------------------------------------------

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jdouble, jint, jstring};

fn jstr(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|v| v.into()).unwrap_or_default()
}

fn respond(env: &mut JNIEnv, value: Value) -> jstring {
    let text = serde_json::to_string(&value)
        .unwrap_or_else(|_| r#"{"ok":false,"error":"serialization"}"#.into());
    match env.new_string(text) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Catch panics so nothing unwinds across the FFI boundary.
fn guarded(env: &mut JNIEnv, f: impl FnOnce() -> Value + std::panic::UnwindSafe) -> jstring {
    let value = std::panic::catch_unwind(f)
        .unwrap_or_else(|_| json!({"ok": false, "error": "native panic"}));
    respond(env, value)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_scanHardware(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded(&mut env, || api_scan(None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_recommend(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded(&mut env, || api_recommend(None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_inspectGguf(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
) -> jstring {
    let path = jstr(&mut env, &path);
    guarded(&mut env, move || api_inspect(&path, None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_runVibeMark(
    mut env: JNIEnv,
    _class: JClass,
    path: JString,
    budget_secs: jint,
) -> jstring {
    let path = jstr(&mut env, &path);
    let budget = budget_secs.max(5) as u64;
    guarded(&mut env, move || api_vibemark(&path, budget, None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_leaderboard(
    mut env: JNIEnv,
    _class: JClass,
    profile: JString,
) -> jstring {
    let profile = jstr(&mut env, &profile);
    guarded(&mut env, move || api_leaderboard(&profile, None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_submitLeaderboard(
    mut env: JNIEnv,
    _class: JClass,
    entry_json: JString,
) -> jstring {
    let entry = jstr(&mut env, &entry_json);
    guarded(&mut env, move || api_submit(&entry, None))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_startServer(
    mut env: JNIEnv,
    _class: JClass,
    port: jint,
    lan: jboolean,
    token: JString,
    data_dir: JString,
) -> jstring {
    let token = jstr(&mut env, &token);
    let data_dir = jstr(&mut env, &data_dir);
    let port = if port <= 0 {
        server::DEFAULT_PORT
    } else {
        port as u16
    };
    let lan = lan != 0;
    guarded(&mut env, move || {
        api_start_server(port, lan, &token, &data_dir)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_stopServer(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded(&mut env, api_stop_server)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_serverStatus(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded(&mut env, api_status)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_chat(
    mut env: JNIEnv,
    _class: JClass,
    prompt: JString,
    strip_think: jboolean,
) -> jstring {
    let prompt = jstr(&mut env, &prompt);
    let strip = strip_think != 0;
    guarded(&mut env, move || api_chat(&prompt, strip))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_unloadModel(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded(&mut env, api_unload)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_setThermalReading(
    mut env: JNIEnv,
    _class: JClass,
    celsius: jdouble,
) -> jstring {
    guarded(&mut env, move || api_set_thermal(celsius))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_setIdleTimeout(
    mut env: JNIEnv,
    _class: JClass,
    seconds: jint,
) -> jstring {
    let seconds = seconds.max(10) as u64;
    guarded(&mut env, move || api_set_idle(seconds))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_vibelocal_app_core_VibeLocalBridge_setDataDir(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
) -> jstring {
    let dir = jstr(&mut env, &data_dir);
    guarded(&mut env, move || api_set_data_dir(&dir))
}

// ---------------------------------------------------------------------------
// C-ABI exports (PC embedding / tests)
// ---------------------------------------------------------------------------

fn to_c_string(value: Value) -> *mut c_char {
    let text = serde_json::to_string(&value).unwrap_or_default();
    CString::new(text)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `ptr` must be a string returned by one of the `vibe_local_*` functions,
/// and must only be freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

/// # Safety
/// `path` must be a valid NUL-terminated C string or null.
unsafe fn cstr_opt(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn vibe_local_scan_hardware() -> *mut c_char {
    to_c_string(api_scan(None))
}

#[unsafe(no_mangle)]
pub extern "C" fn vibe_local_recommend() -> *mut c_char {
    to_c_string(api_recommend(None))
}

/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_inspect_gguf(path: *const c_char) -> *mut c_char {
    let path = unsafe { cstr_opt(path) }.unwrap_or_default();
    to_c_string(api_inspect(&path, None))
}

/// # Safety
/// `path` must be a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_vibemark(path: *const c_char, budget_secs: u32) -> *mut c_char {
    let path = unsafe { cstr_opt(path) }.unwrap_or_default();
    to_c_string(api_vibemark(&path, u64::from(budget_secs.max(5)), None))
}

/// # Safety
/// `profile` must be a valid NUL-terminated C string or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_leaderboard(profile: *const c_char) -> *mut c_char {
    let profile = unsafe { cstr_opt(profile) }.unwrap_or_default();
    to_c_string(api_leaderboard(&profile, None))
}

/// # Safety
/// All pointer arguments must be valid NUL-terminated C strings or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_start_server(
    port: u16,
    lan: u8,
    token: *const c_char,
    data_dir: *const c_char,
) -> *mut c_char {
    let token = unsafe { cstr_opt(token) }.unwrap_or_default();
    let data_dir = unsafe { cstr_opt(data_dir) }
        .unwrap_or_else(|| default_data_dir().to_string_lossy().into_owned());
    let port = if port == 0 {
        server::DEFAULT_PORT
    } else {
        port
    };
    to_c_string(api_start_server(port, lan != 0, &token, &data_dir))
}

#[unsafe(no_mangle)]
pub extern "C" fn vibe_local_stop_server() -> *mut c_char {
    to_c_string(api_stop_server())
}

/// # Safety
/// `prompt` must be a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vibe_local_chat(prompt: *const c_char, strip_think: u8) -> *mut c_char {
    let prompt = unsafe { cstr_opt(prompt) }.unwrap_or_default();
    to_c_string(api_chat(&prompt, strip_think != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The bridge keeps one process-global state; these tests must not run
    // concurrently or the server lifecycle test races the status assertions.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point the bridge at a throwaway data dir shared by all tests (the
    /// global state is created once). Tests hold TEST_LOCK, so mutating the
    /// process environment here cannot race other tests.
    fn temp_data_dir() {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
        unsafe { std::env::set_var("VIBE_DATA_DIR", dir.path()) };
    }

    fn parse(ptr: *mut c_char) -> Value {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { vibe_local_free_string(ptr) };
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn c_abi_scan_and_status() {
        let _guard = TEST_LOCK.lock().unwrap();
        temp_data_dir();
        let scan = parse(vibe_local_scan_hardware());
        assert_eq!(scan["ok"], true);
        assert!(scan["data"]["ram_total_mb"].as_u64().unwrap() > 0);
        let status = parse(to_c_string(api_status()));
        assert_eq!(status["ok"], true);
        assert_eq!(status["data"]["server_running"], false);
    }

    #[test]
    fn c_abi_error_paths() {
        let _guard = TEST_LOCK.lock().unwrap();
        temp_data_dir();
        let bad = unsafe { vibe_local_inspect_gguf(c"/nonexistent/model.gguf".as_ptr()) };
        let v = parse(bad);
        assert_eq!(v["ok"], false);
        assert!(!v["error"].as_str().unwrap().is_empty());
        let bad_temp = parse(to_c_string(api_set_thermal(500.0)));
        assert_eq!(bad_temp["ok"], false);
    }

    #[test]
    fn server_start_stop_via_api() {
        let _guard = TEST_LOCK.lock().unwrap();
        temp_data_dir();
        let dir = tempfile::tempdir().unwrap();
        let started = api_start_server(0, false, "", &dir.path().to_string_lossy());
        assert_eq!(started["ok"], true, "{started}");
        let port = started["data"]["port"].as_u64().unwrap();
        assert!(port > 0);
        let resp = ureq::get(&format!("http://127.0.0.1:{port}/api/status"))
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .unwrap();
        let status: Value = resp.into_json().unwrap();
        assert_eq!(status["ok"], true);
        let stopped = api_stop_server();
        assert_eq!(stopped["ok"], true);
    }
}
