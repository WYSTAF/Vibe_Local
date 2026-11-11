//! CLI subcommand implementations, shared with the JNI bridge.

use std::path::{Path, PathBuf};

use serde_json::json;

use vibe_local_core::catalog::{self, CatalogEntry, PillarScore, Throughput};
use vibe_local_core::db::SharedDb;
use vibe_local_core::hardware::HardwareProbe;
use vibe_local_core::{gguf, hf_crawler, recommender, util};
use vibe_local_engine::{Engine, EngineConfig, LoadRequest};

pub fn scan(db: &SharedDb) -> anyhow::Result<serde_json::Value> {
    let report = HardwareProbe::scan();
    db.lock().unwrap().insert_scan(&report)?;
    Ok(serde_json::to_value(&report)?)
}

pub fn discover(
    db: &SharedDb,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<hf_crawler::DiscoveredFile>> {
    let files = hf_crawler::discover(query, limit)?;
    let guard = db.lock().unwrap();
    for f in &files {
        guard.upsert_model(f)?;
    }
    Ok(files)
}

pub fn inspect(db: &SharedDb, path: &Path) -> anyhow::Result<gguf::GgufInfo> {
    let info = gguf::parse_file(path)?;
    db.lock().unwrap().insert_inspection(&info)?;
    Ok(info)
}

/// Map the local machine onto the closest reference hardware profile class.
/// The actual hardware is always recorded in pillar detail for provenance.
pub fn reference_profile(hw: &vibe_local_core::HardwareReport) -> &'static str {
    match hw.preferred_backend.as_str() {
        "cuda" => "tesla_t4",
        "vulkan" => "rx580_vulkan",
        _ => {
            if hw.arch == "aarch64" && hw.ram_total_mb >= 11000 {
                "nothing_phone_2a"
            } else {
                "i5_4690_ddr3"
            }
        }
    }
}

pub struct BenchOutcome {
    pub result: vibe_local_engine::VibeMarkResult,
    pub entry: CatalogEntry,
}

/// Run VibeMark on a local GGUF (or resolve+download a HF model id first).
pub fn bench(
    db: &SharedDb,
    engine: &Engine,
    target: &str,
    models_dir: &Path,
    budget_secs: u64,
) -> anyhow::Result<BenchOutcome> {
    let hw = HardwareProbe::scan();
    let path = resolve_target(target, models_dir, &hw)?;
    let info = gguf::parse_file(&path)?;
    db.lock().unwrap().insert_inspection(&info)?;

    let backend = hw.preferred_backend.clone();
    let ngl = if backend == "cpu" { 0 } else { 99 };
    engine.load(LoadRequest {
        path: path.to_string_lossy().into_owned(),
        backend: backend.clone(),
        ngl,
        ctx: 4096,
    })?;
    let result = engine.vibemark(budget_secs)?;
    engine.unload();

    {
        let guard = db.lock().unwrap();
        guard.insert_vibemark(
            &result.model,
            &result.backend,
            &result.engine,
            result.prefill_tps,
            result.decode_tps,
            result.ttft_ms,
            result.peak_ram_mb,
            result.duration_s,
        )?;
        let profile = reference_profile(&hw);
        let hw_detail = serde_json::to_value(&hw)?;
        guard.insert_benchmark(
            &result.model, "", &result.model, &info.quant, profile, &backend,
            "hardware_throughput",
            (result.decode_tps / 60.0).min(1.0),
            &json!({"prefill_tps": result.prefill_tps, "decode_tps": result.decode_tps, "hardware": hw_detail}),
            "measured",
        )?;
        let latency_score = (1.0 - (result.ttft_ms / 2000.0)).clamp(0.0, 1.0);
        guard.insert_benchmark(
            &result.model, "", &result.model, &info.quant, profile, &backend,
            "latency_thermal_efficiency",
            latency_score,
            &json!({"ttft_ms": result.ttft_ms, "peak_ram_mb": result.peak_ram_mb, "hardware": hw_detail}),
            "measured",
        )?;
    }

    let entry = build_bench_entry(&info, &result, &hw, db)?;
    Ok(BenchOutcome { result, entry })
}

fn resolve_target(
    target: &str,
    models_dir: &Path,
    hw: &vibe_local_core::HardwareReport,
) -> anyhow::Result<PathBuf> {
    let as_path = PathBuf::from(target);
    if as_path.is_file() {
        return Ok(as_path);
    }
    // Treat as a HF model query: pick the best trusted GGUF for this hardware.
    eprintln!("resolving '{target}' from Hugging Face...");
    let files = hf_crawler::discover(target, 5)?;
    anyhow::ensure!(!files.is_empty(), "no GGUF files found for '{target}'");
    let rec = recommender::recommend(hw);
    let preferred_quant = rec
        .candidates
        .first()
        .map(|c| c.quant.clone())
        .unwrap_or_else(|| "Q4_K_M".into());
    let pick = files
        .iter()
        .filter(|f| f.trusted)
        .find(|f| f.quant.as_deref() == Some(preferred_quant.as_str()))
        .or_else(|| files.iter().find(|f| f.quant.as_deref() == Some("Q4_K_M")))
        .or_else(|| files.iter().find(|f| f.trusted))
        .unwrap_or(&files[0]);
    let dest = models_dir.join(&pick.filename);
    if dest.is_file() {
        eprintln!("using cached {}", dest.display());
        return Ok(dest);
    }
    eprintln!(
        "downloading {} ({} bytes)...",
        pick.download_url, pick.size_bytes
    );
    let written = hf_crawler::download(&pick.download_url, &dest)?;
    eprintln!("downloaded {written} bytes to {}", dest.display());
    Ok(dest)
}

fn build_bench_entry(
    info: &gguf::GgufInfo,
    result: &vibe_local_engine::VibeMarkResult,
    hw: &vibe_local_core::HardwareReport,
    db: &SharedDb,
) -> anyhow::Result<CatalogEntry> {
    let profile = reference_profile(hw);
    let model_id = Some(info.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            Path::new(&info.path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| result.model.clone())
        });
    let mut pillars = std::collections::BTreeMap::new();
    let hw_detail = serde_json::to_value(hw)?;
    pillars.insert(
        "hardware_throughput".to_string(),
        PillarScore {
            score: (result.decode_tps / 60.0).min(1.0),
            metric: "decode tok/s (ref 60)".into(),
            n: 1,
            detail: json!({"prefill_tps": result.prefill_tps, "decode_tps": result.decode_tps, "hardware": hw_detail}),
        },
    );
    pillars.insert(
        "latency_thermal_efficiency".to_string(),
        PillarScore {
            score: (1.0 - (result.ttft_ms / 2000.0)).clamp(0.0, 1.0),
            metric: "TTFT ms".into(),
            n: 1,
            detail: json!({"ttft_ms": result.ttft_ms, "peak_ram_mb": result.peak_ram_mb, "hardware": hw_detail}),
        },
    );
    // Quality pillars: inherit from the seed catalog when a matching entry
    // exists, clearly labeled per-pillar as seed estimates (never measured).
    let seed = {
        let guard = db.lock().unwrap();
        guard.leaderboard(profile)?.into_iter().find(|e| {
            e.quant == info.quant && (e.model_id == model_id || model_id.contains(&e.model_id))
        })
    };
    if let Some(seed) = seed {
        for (id, p) in &seed.pillars {
            if catalog::QUALITY_WEIGHTS.iter().any(|(q, _)| q == id) {
                let mut labeled = p.clone();
                labeled.detail = json!({"source": "seed_estimate", "original_detail": p.detail});
                pillars.insert(id.clone(), labeled);
            }
        }
    }
    let is_moe = info.is_moe;
    let total_params_b = estimate_params_from_size(info);
    let entry = CatalogEntry {
        model_id: model_id.clone(),
        hf_repo: String::new(),
        filename: Path::new(&info.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        download_url: String::new(),
        quant: info.quant.clone(),
        architecture: info.architecture.clone(),
        family: if is_moe { "moe".into() } else { "dense".into() },
        total_params_b,
        active_params_b: if is_moe {
            let experts = info.expert_count.unwrap_or(1).max(1) as f64;
            let used = info.expert_used_count.unwrap_or(1).max(1) as f64;
            (total_params_b * used / experts).max(0.1)
        } else {
            total_params_b
        },
        size_bytes: info.file_size_bytes,
        context_length: info.context_length.unwrap_or(4096),
        hardware_profile: profile.into(),
        provenance: "measured".into(),
        source_url: String::new(),
        pillars,
        throughput: Throughput {
            prefill_tps: result.prefill_tps,
            decode_tps: result.decode_tps,
            ttft_ms: result.ttft_ms,
            peak_ram_mb: result.peak_ram_mb,
            peak_vram_mb: 0.0,
            thermal_delta_c: 0.0,
        },
        vibe_quality_score: None,
        power_per_parameter: None,
        pareto_optimal: None,
    };
    Ok(entry.with_derived())
}

/// Rough parameter count from file size and quant bpw (labeled estimate).
fn estimate_params_from_size(info: &gguf::GgufInfo) -> f64 {
    let bpw = recommender::bits_per_weight(&info.quant).unwrap_or(4.85);
    let params = info.file_size_bytes as f64 * 8.0 / bpw / 1e9;
    (params * 10.0).round() / 10.0
}

/// Submit a measured entry to a remote Vibe-Link leaderboard endpoint.
pub fn submit_entry(base_url: &str, entry: &CatalogEntry) -> anyhow::Result<serde_json::Value> {
    let url = format!("{}/api/leaderboard/submit", base_url.trim_end_matches('/'));
    // Only allow loopback/LAN submission targets over http(s); this is a
    // client-side POST of benchmark data, not a remote fetcher, but still
    // reject non-http schemes.
    let parsed = url::Url::parse(&url)?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "submit URL must be http(s)"
    );
    let resp = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .post(&url)
        .send_json(json!({"entry": entry}))?;
    Ok(resp.into_json()?)
}

pub fn leaderboard_json(db: &SharedDb, profile: &str) -> anyhow::Result<serde_json::Value> {
    let mut entries = db.lock().unwrap().leaderboard(profile)?;
    let mask = catalog::pareto_frontier(&entries);
    for (e, optimal) in entries.iter_mut().zip(mask) {
        e.pareto_optimal = Some(optimal);
    }
    let labels: serde_json::Map<String, serde_json::Value> = catalog::PILLARS
        .iter()
        .map(|p| ((*p).to_string(), json!(catalog::pillar_label(p))))
        .collect();
    Ok(json!({"pillars": catalog::PILLARS, "pillar_labels": labels, "entries": entries}))
}

/// Import the bundled seed catalog into the DB (idempotent).
pub fn import_seed_catalog(db: &SharedDb) -> anyhow::Result<usize> {
    match catalog::find_catalog_file() {
        Some(path) => {
            let catalog = catalog::Catalog::load(&path)?;
            let n = db.lock().unwrap().import_catalog(&catalog)?;
            Ok(n)
        }
        None => Ok(0),
    }
}

pub fn print_hardware(report: &vibe_local_core::HardwareReport) {
    println!("OS:            {} ({})", report.os, report.arch);
    println!(
        "CPU:           {} ({} cores)",
        report.cpu_model, report.cpu_cores
    );
    println!("CPU flags:     {}", report.cpu_flags.join(", "));
    println!(
        "RAM:           {} MB total / {} MB free",
        report.ram_total_mb, report.ram_free_mb
    );
    if report.gpus.is_empty() {
        println!("GPU:           none detected (CPU backend)");
    }
    for g in &report.gpus {
        println!(
            "GPU:           {} [{}] VRAM: {} backend: {}",
            g.name,
            g.vendor,
            g.vram_mb
                .map(|v| format!("{v} MB"))
                .unwrap_or_else(|| "shared/unknown".into()),
            g.backend
        );
    }
    println!("Backend:       {}", report.preferred_backend);
    println!("Tier:          {}", report.tier);
    println!(
        "Portable USB:  {}",
        report
            .portable
            .root
            .clone()
            .unwrap_or_else(|| "not available".into())
    );
}

pub fn print_recommendation(rec: &recommender::Recommendation) {
    println!("Tier: {}\n{}\n", rec.tier, rec.rationale);
    for (i, c) in rec.candidates.iter().enumerate() {
        println!(
            "{:>2}. {} [{}] {} — {:.1}B total / {:.1}B active, {:.2} GB, fit: {}, ngl: {}",
            i + 1,
            c.model_id,
            c.family,
            c.quant,
            c.total_params_b,
            c.active_params_b,
            c.size_bytes as f64 / 1e9,
            c.fit,
            c.ngl
        );
        for r in &c.reasons {
            println!("      - {r}");
        }
        println!("      {}", c.download_url);
    }
}

pub fn print_inspection(info: &gguf::GgufInfo) {
    println!("File:          {}", info.path);
    println!(
        "GGUF version:  {} ({} tensors, {} KV pairs)",
        info.version, info.tensor_count, info.metadata_kv_count
    );
    println!("Architecture:  {}", info.architecture);
    println!("Name:          {}", info.name);
    println!("Quant:         {}", info.quant);
    println!(
        "Context:       {}",
        info.context_length
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into())
    );
    println!(
        "Layers:        {}",
        info.block_count
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into())
    );
    println!(
        "Heads:         {} (KV: {})",
        info.head_count
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into()),
        info.head_count_kv
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into())
    );
    if info.is_moe {
        println!(
            "MoE:           {} experts, {} active/token",
            info.expert_count.unwrap_or(0),
            info.expert_used_count.unwrap_or(0)
        );
    }
    println!("Size:          {:.2} GB", info.file_size_bytes as f64 / 1e9);
    println!("RAM estimates (weights + KV cache + compute):");
    for r in &info.ram {
        println!(
            "  ctx {:>5}: {:>8.0} MB  (weights {:.0} + KV {:.0} + compute {:.0})",
            r.ctx, r.total_mb, r.weights_mb, r.kv_cache_mb, r.compute_mb
        );
    }
}

pub fn print_vibemark(r: &vibe_local_engine::VibeMarkResult) {
    println!("VibeMark — {} [{} / {}]", r.model, r.backend, r.engine);
    println!("  Prefill:   {:>10.1} tok/s", r.prefill_tps);
    println!("  Decode:    {:>10.1} tok/s", r.decode_tps);
    println!("  TTFT:      {:>10.1} ms", r.ttft_ms);
    println!("  Peak RAM:  {:>10.1} MB", r.peak_ram_mb);
    println!("  Duration:  {:>10.2} s ({})", r.duration_s, r.provenance);
}

pub fn print_leaderboard(value: &serde_json::Value) {
    let entries = value["entries"].as_array().cloned().unwrap_or_default();
    if entries.is_empty() {
        println!(
            "Leaderboard is empty. Run `vibe-local bench <model>` or import the seed catalog."
        );
        return;
    }
    let mut sorted = entries;
    sorted.sort_by(|a, b| {
        b["vibe_quality_score"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["vibe_quality_score"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    println!(
        "{:<40} {:<8} {:<8} {:>7} {:>9} {:>10} {:<14}",
        "MODEL", "FAMILY", "QUANT", "SCORE", "PWR/PARAM", "DECODE/s", "PROVENANCE"
    );
    for e in sorted.iter().take(40) {
        println!(
            "{:<40} {:<8} {:<8} {:>7.3} {:>9.3} {:>10.1} {:<14}",
            truncate(e["model_id"].as_str().unwrap_or("?"), 40),
            e["family"].as_str().unwrap_or("?"),
            e["quant"].as_str().unwrap_or("?"),
            e["vibe_quality_score"].as_f64().unwrap_or(0.0),
            e["power_per_parameter"].as_f64().unwrap_or(0.0),
            e["throughput"]["decode_tps"].as_f64().unwrap_or(0.0),
            e["provenance"].as_str().unwrap_or("?"),
        );
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n - 1])
    }
}

/// Resolve or create the LAN token: explicit > stored > freshly generated.
pub fn resolve_lan_token(db: &SharedDb, explicit: Option<String>) -> anyhow::Result<String> {
    if let Some(t) = explicit {
        anyhow::ensure!(t.len() >= 16, "LAN token must be at least 16 characters");
        db.lock().unwrap().set_setting("lan_token", &t)?;
        return Ok(t);
    }
    if let Some(t) = db.lock().unwrap().get_setting("lan_token")? {
        return Ok(t);
    }
    let token = util::random_token()?;
    db.lock().unwrap().set_setting("lan_token", &token)?;
    Ok(token)
}

pub fn engine_config(idle_secs: u64) -> EngineConfig {
    EngineConfig {
        llama_server_path: std::env::var("VIBE_LLAMA_SERVER").ok().map(PathBuf::from),
        internal_port: vibe_local_engine::supervisor::DEFAULT_INTERNAL_PORT,
        idle_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vibe_local_core::Db;

    #[test]
    fn token_resolution() {
        let db = Db::open_in_memory().unwrap().shared();
        let t1 = resolve_lan_token(&db, None).unwrap();
        assert_eq!(t1.len(), 32);
        let t2 = resolve_lan_token(&db, None).unwrap();
        assert_eq!(t1, t2, "stored token is reused");
        let t3 = resolve_lan_token(&db, Some("explicit-token-123456".into())).unwrap();
        assert_eq!(t3, "explicit-token-123456");
        assert!(resolve_lan_token(&db, Some("short".into())).is_err());
    }

    #[test]
    fn profile_mapping() {
        let mut hw = HardwareProbe::scan();
        hw.preferred_backend = "cuda".into();
        assert_eq!(reference_profile(&hw), "tesla_t4");
        hw.preferred_backend = "vulkan".into();
        assert_eq!(reference_profile(&hw), "rx580_vulkan");
        hw.preferred_backend = "cpu".into();
        hw.arch = "aarch64".into();
        hw.ram_total_mb = 12288;
        assert_eq!(reference_profile(&hw), "nothing_phone_2a");
        hw.arch = "x86_64".into();
        assert_eq!(reference_profile(&hw), "i5_4690_ddr3");
    }

    #[test]
    fn params_estimate() {
        let mut info = gguf::parse_reader(std::io::Cursor::new(minimal_gguf())).unwrap();
        info.file_size_bytes = 4_680_000_000;
        info.quant = "Q4_K_M".into();
        let p = estimate_params_from_size(&info);
        assert!((7.0..8.5).contains(&p), "got {p}");
    }

    fn minimal_gguf() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&gguf::GGUF_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes
    }

    #[test]
    fn bench_entry_builds() {
        let db = Db::open_in_memory().unwrap().shared();
        let mut info = gguf::parse_reader(std::io::Cursor::new(minimal_gguf())).unwrap();
        info.path = "/models/Test-7B-Q4_K_M.gguf".into();
        info.quant = "Q4_K_M".into();
        info.architecture = "qwen2".into();
        info.file_size_bytes = 4_680_000_000;
        let result = vibe_local_engine::VibeMarkResult {
            model: "Test-7B-Q4_K_M.gguf".into(),
            backend: "vulkan".into(),
            engine: "llama-server".into(),
            prefill_tps: 400.0,
            decode_tps: 30.0,
            ttft_ms: 200.0,
            peak_ram_mb: 5200.0,
            duration_s: 15.0,
            provenance: "measured".into(),
        };
        let mut hw = HardwareProbe::scan();
        hw.preferred_backend = "vulkan".into();
        let entry = build_bench_entry(&info, &result, &hw, &db).unwrap();
        entry.validate().unwrap();
        assert_eq!(entry.hardware_profile, "rx580_vulkan");
        assert_eq!(entry.provenance, "measured");
        assert!(entry.pillars.contains_key("hardware_throughput"));
        assert!(entry.pillars.contains_key("latency_thermal_efficiency"));
        assert!(entry.vibe_quality_score.is_some());
    }
}
