//! 10-pillar benchmark catalog: scoring math, Power-per-Parameter metric,
//! Pareto frontier solver and the shared entry schema used by the Rust
//! server, the Python Kaggle runner and the web leaderboard.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PILLARS: [&str; 10] = [
    "code_generation",
    "systems_polyglot",
    "math_reasoning",
    "hard_science_gpqa",
    "instruction_schema",
    "long_context_needle",
    "truthfulness_anti_sycophancy",
    "quant_retention_kl",
    "hardware_throughput",
    "latency_thermal_efficiency",
];

pub fn pillar_label(id: &str) -> &'static str {
    match id {
        "code_generation" => "Code Generation (HumanEval+/LiveCodeBench)",
        "systems_polyglot" => "Systems Polyglot (MultiPL-E Rust/C++/TS)",
        "math_reasoning" => "Math Reasoning (GSM8K/MATH)",
        "hard_science_gpqa" => "Hard Science (GPQA Diamond)",
        "instruction_schema" => "Instruction & Schema (IFEval + FNV-1a anchors)",
        "long_context_needle" => "Long Context Needle (4k-16k)",
        "truthfulness_anti_sycophancy" => "Truthfulness / Anti-Sycophancy (TruthfulQA)",
        "quant_retention_kl" => "Quant Retention (KL vs Q8_0/FP16)",
        "hardware_throughput" => "Hardware Throughput (prefill/decode tok/s)",
        "latency_thermal_efficiency" => "Latency & Thermal Efficiency",
        _ => "Unknown pillar",
    }
}

/// Weights of the 8 quality pillars in `vibe_quality_score` (sum = 1.0).
pub const QUALITY_WEIGHTS: [(&str, f64); 8] = [
    ("code_generation", 0.16),
    ("systems_polyglot", 0.12),
    ("math_reasoning", 0.14),
    ("hard_science_gpqa", 0.10),
    ("instruction_schema", 0.12),
    ("long_context_needle", 0.12),
    ("truthfulness_anti_sycophancy", 0.12),
    ("quant_retention_kl", 0.12),
];

pub const QUANT_ORDER: [&str; 8] = [
    "Q8_0", "Q6_K", "Q5_K_M", "IQ4_NL", "Q4_K_M", "IQ4_XS", "Q3_K_M", "Q2_K",
];

pub fn quant_rank(quant: &str) -> usize {
    QUANT_ORDER
        .iter()
        .position(|q| *q == quant)
        .unwrap_or(QUANT_ORDER.len())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PillarScore {
    pub score: f64,
    pub metric: String,
    #[serde(default)]
    pub n: u64,
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Throughput {
    #[serde(default)]
    pub prefill_tps: f64,
    #[serde(default)]
    pub decode_tps: f64,
    #[serde(default)]
    pub ttft_ms: f64,
    #[serde(default)]
    pub peak_ram_mb: f64,
    #[serde(default)]
    pub peak_vram_mb: f64,
    #[serde(default)]
    pub thermal_delta_c: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub model_id: String,
    #[serde(default)]
    pub hf_repo: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub download_url: String,
    pub quant: String,
    #[serde(default)]
    pub architecture: String,
    pub family: String,
    pub total_params_b: f64,
    pub active_params_b: f64,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub context_length: u64,
    pub hardware_profile: String,
    /// "seed_estimate" | "measured" | "submitted"
    pub provenance: String,
    #[serde(default)]
    pub source_url: String,
    pub pillars: BTreeMap<String, PillarScore>,
    #[serde(default)]
    pub throughput: Throughput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibe_quality_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_per_parameter: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pareto_optimal: Option<bool>,
}

impl CatalogEntry {
    /// Weighted mean of the 8 quality pillars (missing pillars count as 0).
    pub fn compute_quality_score(&self) -> f64 {
        QUALITY_WEIGHTS
            .iter()
            .map(|(id, w)| {
                w * self
                    .pillars
                    .get(*id)
                    .map(|p| p.score.clamp(0.0, 1.0))
                    .unwrap_or(0.0)
            })
            .sum()
    }

    pub fn compute_power_per_parameter(&self) -> f64 {
        if self.active_params_b <= 0.0 {
            return 0.0;
        }
        self.compute_quality_score() / self.active_params_b
    }

    /// Fill derived fields (quality score, power/param) in place.
    pub fn with_derived(mut self) -> Self {
        self.vibe_quality_score = Some(self.compute_quality_score());
        self.power_per_parameter = Some(self.compute_power_per_parameter());
        self
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.model_id.trim().is_empty() {
            anyhow::bail!("model_id is required");
        }
        if self.family != "moe" && self.family != "dense" {
            anyhow::bail!("family must be 'moe' or 'dense'");
        }
        if self.active_params_b <= 0.0 || self.total_params_b <= 0.0 {
            anyhow::bail!("parameter counts must be positive");
        }
        if self.active_params_b > self.total_params_b + 1e-9 {
            anyhow::bail!("active params cannot exceed total params");
        }
        if !["seed_estimate", "measured", "submitted"].contains(&self.provenance.as_str()) {
            anyhow::bail!("invalid provenance '{}'", self.provenance);
        }
        if !self.download_url.is_empty()
            && !self.download_url.starts_with("https://huggingface.co/")
        {
            anyhow::bail!("download_url must be a huggingface.co https URL");
        }
        for (id, p) in &self.pillars {
            if !PILLARS.contains(&id.as_str()) {
                anyhow::bail!("unknown pillar '{id}'");
            }
            if !(0.0..=1.0).contains(&p.score) || !p.score.is_finite() {
                anyhow::bail!("pillar '{id}' score out of range: {}", p.score);
            }
        }
        Ok(())
    }
}

/// Pareto frontier over (active_params_b minimize, quality maximize).
/// Returns a boolean mask aligned with `entries`.
pub fn pareto_frontier(entries: &[CatalogEntry]) -> Vec<bool> {
    let scores: Vec<f64> = entries.iter().map(|e| e.compute_quality_score()).collect();
    let mut mask = vec![true; entries.len()];
    for i in 0..entries.len() {
        for j in 0..entries.len() {
            if i == j {
                continue;
            }
            let dominates = entries[j].active_params_b <= entries[i].active_params_b
                && scores[j] >= scores[i]
                && (entries[j].active_params_b < entries[i].active_params_b
                    || scores[j] > scores[i]);
            if dominates {
                mask[i] = false;
                break;
            }
        }
    }
    mask
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub label: String,
    pub backend: String,
    #[serde(default)]
    pub vram_mb: u64,
    #[serde(default)]
    pub ram_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub provenance_note: String,
    pub pillars: Vec<String>,
    #[serde(default)]
    pub pillar_labels: BTreeMap<String, String>,
    pub hardware_profiles: BTreeMap<String, HardwareProfile>,
    pub entries: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Catalog> {
        let text = std::fs::read_to_string(path)?;
        let catalog: Catalog = serde_json::from_str(&text)?;
        if catalog.schema_version != 1 {
            anyhow::bail!(
                "unsupported catalog schema_version {}",
                catalog.schema_version
            );
        }
        Ok(catalog)
    }

    pub fn entries_for_profile(&self, profile: &str) -> Vec<CatalogEntry> {
        self.entries
            .iter()
            .filter(|e| profile.is_empty() || e.hardware_profile == profile)
            .cloned()
            .map(CatalogEntry::with_derived)
            .collect()
    }
}

/// Locate `vibe_benchmark_catalog.json` near the executable or working dir.
pub fn find_catalog_file() -> Option<std::path::PathBuf> {
    let mut bases: Vec<std::path::PathBuf> = Vec::new();
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
        let candidate = base.join("benchmarks").join("vibe_benchmark_catalog.json");
        if candidate.is_file() {
            return candidate.canonicalize().ok().or(Some(candidate));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str, active: f64, scores: &[f64]) -> CatalogEntry {
        let mut pillars = BTreeMap::new();
        for (i, id) in PILLARS.iter().enumerate() {
            pillars.insert(
                (*id).to_string(),
                PillarScore {
                    score: scores.get(i).copied().unwrap_or(0.5),
                    metric: "test".into(),
                    n: 10,
                    detail: serde_json::Value::Null,
                },
            );
        }
        CatalogEntry {
            model_id: model.into(),
            hf_repo: format!("bartowski/{model}-GGUF"),
            filename: format!("{model}-Q4_K_M.gguf"),
            download_url: format!(
                "https://huggingface.co/bartowski/{model}-GGUF/resolve/main/{model}-Q4_K_M.gguf"
            ),
            quant: "Q4_K_M".into(),
            architecture: "qwen2".into(),
            family: "dense".into(),
            total_params_b: active,
            active_params_b: active,
            size_bytes: 4_680_000_000,
            context_length: 32768,
            hardware_profile: "rx580_vulkan".into(),
            provenance: "seed_estimate".into(),
            source_url: String::new(),
            pillars,
            throughput: Throughput::default(),
            vibe_quality_score: None,
            power_per_parameter: None,
            pareto_optimal: None,
        }
    }

    #[test]
    fn weights_sum_to_one() {
        let sum: f64 = QUALITY_WEIGHTS.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-9);
        assert_eq!(PILLARS.len(), 10);
        assert_eq!(QUALITY_WEIGHTS.len(), 8);
    }

    #[test]
    fn quality_score_math() {
        let e = entry("M", 7.6, &[1.0; 10]);
        assert!((e.compute_quality_score() - 1.0).abs() < 1e-9);
        let mut e2 = entry("M", 7.6, &[0.0; 10]);
        e2.pillars.get_mut("code_generation").unwrap().score = 1.0;
        assert!((e2.compute_quality_score() - 0.16).abs() < 1e-9);
        // power per parameter
        let e3 = entry("M", 2.0, &[0.8; 10]);
        assert!((e3.compute_power_per_parameter() - 0.4).abs() < 1e-9);
    }

    #[test]
    fn pareto_frontier_correct() {
        // A: small+good (optimal), B: big+good (optimal), C: big+bad (dominated by A),
        // D: same size as A but worse (dominated).
        let a = entry("A", 1.0, &[0.9; 10]);
        let b = entry("B", 7.0, &[0.95; 10]);
        let c = entry("C", 7.0, &[0.5; 10]);
        let d = entry("D", 1.0, &[0.8; 10]);
        let mask = pareto_frontier(&[a, b, c, d]);
        assert_eq!(mask, vec![true, true, false, false]);
    }

    #[test]
    fn quant_ordering() {
        assert!(quant_rank("Q8_0") < quant_rank("Q4_K_M"));
        assert!(quant_rank("Q4_K_M") < quant_rank("Q2_K"));
        assert_eq!(quant_rank("BOGUS"), QUANT_ORDER.len());
    }

    #[test]
    fn validation() {
        let good = entry("M", 7.6, &[0.7; 10]);
        assert!(good.validate().is_ok());
        let mut bad = good.clone();
        bad.family = "transformer".into();
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.pillars.get_mut("code_generation").unwrap().score = 1.5;
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.download_url = "https://evil.com/x.gguf".into();
        assert!(bad.validate().is_err());
        let mut bad = good.clone();
        bad.active_params_b = 99.0;
        assert!(bad.validate().is_err());
        let mut bad = good;
        bad.provenance = "fabricated".into();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn derived_fields() {
        let e = entry("M", 1.3, &[0.7; 10]).with_derived();
        assert!(e.vibe_quality_score.is_some());
        assert!((e.power_per_parameter.unwrap() - 0.7 / 1.3).abs() < 1e-9);
    }
}
