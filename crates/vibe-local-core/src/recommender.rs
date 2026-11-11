//! Hardware-aware MoE vs. Dense decision engine.
//!
//! Tiers:
//! - `gpu_dense`: discrete Vulkan/CUDA GPU with >=6GB fast VRAM -> dense 7B-8B
//!   at Q4_K_M/Q5_K_M fully offloaded (`-ngl 99`).
//! - `moe_sparse`: high-RAM but compute/bandwidth-constrained (CPU-only PC,
//!   phones with 8GB+ RAM) -> sparse MoE whose total weights fit in RAM while
//!   active params per token stay small for faster, cooler decode.
//! - `edge_compact`: under 2.5GB free RAM -> compact dense 1.5B-2B at
//!   Q4_K_M/IQ4_XS to avoid Android LMKD kills.

use serde::{Deserialize, Serialize};

use crate::hardware::HardwareReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    GpuDense,
    MoeSparse,
    EdgeCompact,
}

impl Tier {
    pub fn id(&self) -> &'static str {
        match self {
            Tier::GpuDense => "gpu_dense",
            Tier::MoeSparse => "moe_sparse",
            Tier::EdgeCompact => "edge_compact",
        }
    }
}

pub fn classify_tier(hw: &HardwareReport) -> Tier {
    let gpu = hw
        .gpus
        .iter()
        .filter(|g| g.backend == "vulkan" || g.backend == "cuda")
        .filter_map(|g| g.vram_mb.map(|v| (g, v)))
        .max_by_key(|(_, v)| *v);
    if let Some((_, vram)) = gpu
        && vram >= 6144
    {
        return Tier::GpuDense;
    }
    if hw.ram_free_mb < 2560 {
        return Tier::EdgeCompact;
    }
    if hw.ram_total_mb >= 8192 {
        return Tier::MoeSparse;
    }
    Tier::EdgeCompact
}

/// Bytes per weight for common GGUF quantizations (llama.cpp k-quant mixes).
pub fn bits_per_weight(quant: &str) -> Option<f64> {
    Some(match quant {
        "Q2_K" => 2.6,
        "Q3_K_S" => 3.4,
        "Q3_K_M" => 3.9,
        "Q3_K_L" => 4.3,
        "IQ4_XS" => 4.25,
        "IQ4_NL" => 4.5,
        "Q4_K_S" => 4.6,
        "Q4_K_M" => 4.85,
        "Q5_K_S" => 5.5,
        "Q5_K_M" => 5.7,
        "Q6_K" => 6.6,
        "Q8_0" => 8.5,
        "F16" | "BF16" => 16.0,
        _ => return None,
    })
}

/// Estimated GGUF file size for a model at a quantization.
pub fn estimate_size_bytes(total_params_b: f64, quant: &str) -> Option<u64> {
    let bpw = bits_per_weight(quant)?;
    Some((total_params_b * 1e9 * bpw / 8.0) as u64)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub model_id: String,
    pub hf_repo: String,
    pub family: String, // "moe" | "dense"
    pub architecture: String,
    pub total_params_b: f64,
    pub active_params_b: f64,
    pub context_length: u64,
    pub quants: Vec<String>,
}

/// Curated registry of well-known instruct models with public parameter
/// counts. Sizes are derived from quant bits-per-weight (estimates).
pub fn model_registry() -> Vec<ModelSpec> {
    let moe = |id: &str, repo: &str, arch: &str, total: f64, active: f64, ctx: u64| ModelSpec {
        model_id: id.into(),
        hf_repo: repo.into(),
        family: "moe".into(),
        architecture: arch.into(),
        total_params_b: total,
        active_params_b: active,
        context_length: ctx,
        quants: vec![
            "Q3_K_M".into(),
            "IQ4_XS".into(),
            "Q4_K_M".into(),
            "Q5_K_M".into(),
            "Q8_0".into(),
        ],
    };
    let dense = |id: &str, repo: &str, arch: &str, total: f64, ctx: u64| ModelSpec {
        model_id: id.into(),
        hf_repo: repo.into(),
        family: "dense".into(),
        architecture: arch.into(),
        total_params_b: total,
        active_params_b: total,
        context_length: ctx,
        quants: vec![
            "IQ4_XS".into(),
            "Q4_K_M".into(),
            "Q5_K_M".into(),
            "Q6_K".into(),
            "Q8_0".into(),
        ],
    };
    vec![
        // Sparse MoE candidates (active params per token in brackets).
        moe(
            "OLMoE-1B-7B-0924-Instruct",
            "bartowski/OLMoE-1B-7B-0924-Instruct-GGUF",
            "olmoe",
            6.9,
            1.3,
            4096,
        ),
        moe(
            "Granite-3.1-3B-A800M-Instruct",
            "bartowski/granite-3.1-3b-a800m-instruct-GGUF",
            "granitemoe",
            3.3,
            0.8,
            131072,
        ),
        moe(
            "Qwen1.5-MoE-A2.7B-Chat",
            "bartowski/Qwen1.5-MoE-A2.7B-Chat-GGUF",
            "qwen2moe",
            14.3,
            2.7,
            8192,
        ),
        // Dense GPU-tier candidates.
        dense(
            "Qwen2.5-Coder-7B-Instruct",
            "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
            "qwen2",
            7.6,
            32768,
        ),
        dense(
            "DeepSeek-R1-Distill-Qwen-7B",
            "bartowski/DeepSeek-R1-Distill-Qwen-7B-GGUF",
            "qwen2",
            7.6,
            131072,
        ),
        dense(
            "Llama-3.1-8B-Instruct",
            "unsloth/Llama-3.1-8B-Instruct-GGUF",
            "llama",
            8.0,
            131072,
        ),
        // Mid-size dense.
        dense(
            "Qwen2.5-Coder-3B-Instruct",
            "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF",
            "qwen2",
            3.1,
            32768,
        ),
        dense(
            "Llama-3.2-3B-Instruct",
            "bartowski/Llama-3.2-3B-Instruct-GGUF",
            "llama",
            3.2,
            131072,
        ),
        // Compact edge candidates.
        dense(
            "Qwen2.5-Coder-1.5B-Instruct",
            "bartowski/Qwen2.5-Coder-1.5B-Instruct-GGUF",
            "qwen2",
            1.5,
            32768,
        ),
        dense(
            "DeepSeek-R1-Distill-Qwen-1.5B",
            "unsloth/DeepSeek-R1-Distill-Qwen-1.5B-GGUF",
            "qwen2",
            1.5,
            131072,
        ),
        dense(
            "SmolLM2-1.7B-Instruct",
            "bartowski/SmolLM2-1.7B-Instruct-GGUF",
            "llama",
            1.7,
            8192,
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub model_id: String,
    pub hf_repo: String,
    pub filename: String,
    pub quant: String,
    pub family: String,
    pub total_params_b: f64,
    pub active_params_b: f64,
    pub size_bytes: u64,
    pub download_url: String,
    pub est_ram_mb: u64,
    pub fit: String, // "vram" | "ram"
    pub ngl: u32,
    pub reasons: Vec<String>,
    pub est_speedup_vs_dense_7b: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub tier: String,
    pub rationale: String,
    pub candidates: Vec<Candidate>,
}

/// KV-cache + compute overhead estimate (MiB) for a context length.
pub fn runtime_overhead_mb(spec: &ModelSpec, ctx: u64) -> u64 {
    // Conservative: 2 bytes/elem f16 KV, ~32 layers scaled by size, plus
    // compute buffers. Deliberately generous so fits are safe.
    let layers = (spec.total_params_b * 4.0).clamp(16.0, 48.0);
    let kv_per_token = 2.0 * layers * 1024.0 * 2.0; // bytes, ~1024 kv dim
    let kv_mb = kv_per_token * ctx as f64 / (1024.0 * 1024.0);
    (kv_mb + 384.0) as u64
}

pub fn recommend(hw: &HardwareReport) -> Recommendation {
    let tier = classify_tier(hw);
    let registry = model_registry();
    let mut candidates = Vec::new();
    let rationale = match tier {
        Tier::GpuDense => {
            let vram = hw
                .gpus
                .iter()
                .filter(|g| g.backend == "vulkan" || g.backend == "cuda")
                .filter_map(|g| g.vram_mb)
                .max()
                .unwrap_or(0);
            for spec in registry
                .iter()
                .filter(|s| s.family == "dense" && s.total_params_b >= 7.0)
            {
                for quant in ["Q4_K_M", "Q5_K_M"] {
                    let Some(size) = estimate_size_bytes(spec.total_params_b, quant) else {
                        continue;
                    };
                    let size_mb = size / (1024 * 1024);
                    let overhead = runtime_overhead_mb(spec, 4096);
                    if size_mb + overhead + 512 > vram {
                        continue;
                    }
                    candidates.push(Candidate {
                        filename: format!("{}-{quant}.gguf", spec.model_id),
                        download_url: format!(
                            "https://huggingface.co/{}/resolve/main/{}-{quant}.gguf",
                            spec.hf_repo, spec.model_id
                        ),
                        est_ram_mb: size_mb + overhead,
                        fit: "vram".into(),
                        ngl: 99,
                        reasons: vec![
                            format!("Fits 100% in {vram}MB VRAM at {quant} ({size_mb}MB weights + {overhead}MB KV/compute)"),
                            "Full GPU offload (-ngl 99) maximizes decode throughput".into(),
                            format!("{} dense architecture, strong instruct/coding quality", spec.architecture),
                        ],
                        est_speedup_vs_dense_7b: 1.0,
                        model_id: spec.model_id.clone(),
                        hf_repo: spec.hf_repo.clone(),
                        quant: quant.into(),
                        family: spec.family.clone(),
                        total_params_b: spec.total_params_b,
                        active_params_b: spec.active_params_b,
                        size_bytes: size,
                    });
                }
            }
            format!(
                "Discrete {} GPU with {vram}MB fast VRAM detected: dense 7B-8B models at Q4_K_M/Q5_K_M fit fully in VRAM with -ngl 99.",
                hw.preferred_backend
            )
        }
        Tier::MoeSparse => {
            for spec in registry.iter().filter(|s| s.family == "moe") {
                for quant in &spec.quants {
                    if quant == "Q8_0" {
                        continue; // keep MoE footprint lean on bandwidth-bound devices
                    }
                    let Some(size) = estimate_size_bytes(spec.total_params_b, quant) else {
                        continue;
                    };
                    let size_mb = size / (1024 * 1024);
                    let overhead = runtime_overhead_mb(spec, 4096);
                    if size_mb + overhead + 1024 > hw.ram_free_mb {
                        continue;
                    }
                    let speedup = (7.6 / spec.active_params_b).clamp(1.0, 3.0);
                    candidates.push(Candidate {
                        filename: format!("{}-{quant}.gguf", spec.model_id),
                        download_url: format!(
                            "https://huggingface.co/{}/resolve/main/{}-{quant}.gguf",
                            spec.hf_repo, spec.model_id
                        ),
                        est_ram_mb: size_mb + overhead,
                        fit: "ram".into(),
                        ngl: 0,
                        reasons: vec![
                            format!(
                                "Sparse MoE: {:.1}B total weights fit in RAM, only {:.1}B active per token",
                                spec.total_params_b, spec.active_params_b
                            ),
                            format!("~{speedup:.1}x faster decode vs dense 7B on bandwidth-bound hardware, lower heat"),
                            format!("{quant} keeps the full expert set resident ({size_mb}MB)"),
                        ],
                        est_speedup_vs_dense_7b: speedup,
                        model_id: spec.model_id.clone(),
                        hf_repo: spec.hf_repo.clone(),
                        quant: quant.clone(),
                        family: spec.family.clone(),
                        total_params_b: spec.total_params_b,
                        active_params_b: spec.active_params_b,
                        size_bytes: size,
                    });
                }
            }
            format!(
                "High RAM ({}MB total, {}MB free) but compute/bandwidth-constrained ({}): sparse MoE models keep total weights resident while activating only 0.8B-2.7B params per token for 2x-3x faster, cooler decode.",
                hw.ram_total_mb, hw.ram_free_mb, hw.preferred_backend
            )
        }
        Tier::EdgeCompact => {
            for spec in registry
                .iter()
                .filter(|s| s.family == "dense" && s.total_params_b <= 2.0)
            {
                for quant in ["Q4_K_M", "IQ4_XS"] {
                    let Some(size) = estimate_size_bytes(spec.total_params_b, quant) else {
                        continue;
                    };
                    let size_mb = size / (1024 * 1024);
                    let overhead = runtime_overhead_mb(spec, 2048);
                    if size_mb + overhead + 256 > hw.ram_free_mb {
                        continue;
                    }
                    candidates.push(Candidate {
                        filename: format!("{}-{quant}.gguf", spec.model_id),
                        download_url: format!(
                            "https://huggingface.co/{}/resolve/main/{}-{quant}.gguf",
                            spec.hf_repo, spec.model_id
                        ),
                        est_ram_mb: size_mb + overhead,
                        fit: "ram".into(),
                        ngl: 0,
                        reasons: vec![
                            format!("Compact {:.1}B dense model avoids LMKD kills with only {}MB free RAM", spec.total_params_b, hw.ram_free_mb),
                            format!("{quant} footprint: {size_mb}MB weights + {overhead}MB runtime"),
                        ],
                        est_speedup_vs_dense_7b: (7.6 / spec.active_params_b).min(3.0),
                        model_id: spec.model_id.clone(),
                        hf_repo: spec.hf_repo.clone(),
                        quant: quant.into(),
                        family: spec.family.clone(),
                        total_params_b: spec.total_params_b,
                        active_params_b: spec.active_params_b,
                        size_bytes: size,
                    });
                }
            }
            format!(
                "Only {}MB free RAM (<2.5GB): compact dense 1.5B-2B models at Q4_K_M/IQ4_XS prevent Android LMKD kills and desktop OOM.",
                hw.ram_free_mb
            )
        }
    };
    // Best candidates first: smaller active set, then smaller footprint.
    candidates.sort_by(|a, b| {
        a.active_params_b
            .partial_cmp(&b.active_params_b)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.size_bytes.cmp(&b.size_bytes))
    });
    candidates.truncate(12);
    Recommendation {
        tier: tier.id().into(),
        rationale,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{GpuInfo, PortableInfo};

    fn hw(ram_total: u64, ram_free: u64, gpus: Vec<GpuInfo>, backend: &str) -> HardwareReport {
        HardwareReport {
            os: "linux".into(),
            arch: "x86_64".into(),
            cpu_model: "test".into(),
            cpu_cores: 8,
            cpu_flags: vec!["avx2".into()],
            ram_total_mb: ram_total,
            ram_free_mb: ram_free,
            gpus,
            preferred_backend: backend.into(),
            tier: String::new(),
            portable: PortableInfo {
                available: false,
                root: None,
            },
        }
    }

    fn gpu(vram: u64, backend: &str) -> GpuInfo {
        GpuInfo {
            name: "Test GPU".into(),
            vendor: "amd".into(),
            vram_mb: Some(vram),
            backend: backend.into(),
        }
    }

    #[test]
    fn nothing_phone_2a_gets_moe() {
        // Nothing Phone 2a: 12GB LPDDR5, no discrete GPU, CPU NEON.
        let report = hw(12288, 7000, vec![], "cpu");
        assert_eq!(classify_tier(&report), Tier::MoeSparse);
        let rec = recommend(&report);
        assert!(rec.candidates.iter().all(|c| c.family == "moe"));
        assert!(rec.candidates.iter().any(|c| c.model_id.contains("OLMoE")));
        assert!(
            rec.candidates
                .iter()
                .any(|c| c.model_id.contains("Granite"))
        );
        assert!(rec.candidates.iter().all(|c| c.est_ram_mb <= 7000));
    }

    #[test]
    fn rx580_gets_dense_gpu() {
        let report = hw(16384, 12000, vec![gpu(8192, "vulkan")], "vulkan");
        assert_eq!(classify_tier(&report), Tier::GpuDense);
        let rec = recommend(&report);
        assert!(
            rec.candidates
                .iter()
                .all(|c| c.family == "dense" && c.fit == "vram" && c.ngl == 99)
        );
        assert!(
            rec.candidates
                .iter()
                .any(|c| c.model_id == "Qwen2.5-Coder-7B-Instruct" && c.quant == "Q4_K_M")
        );
        // Q5_K_M 8B (~5.7GB) + overhead must still fit 8GB.
        assert!(
            rec.candidates
                .iter()
                .all(|c| c.size_bytes / (1024 * 1024) < 8192)
        );
    }

    #[test]
    fn i5_ddr3_cpu_only_gets_moe() {
        // i5-4690 + 16GB DDR3, no GPU.
        let report = hw(16384, 10000, vec![], "cpu");
        assert_eq!(classify_tier(&report), Tier::MoeSparse);
        let rec = recommend(&report);
        assert!(
            rec.candidates
                .iter()
                .any(|c| c.family == "moe" && c.est_speedup_vs_dense_7b >= 2.0)
        );
    }

    #[test]
    fn low_ram_gets_compact_edge() {
        let report = hw(4096, 2000, vec![], "cpu");
        assert_eq!(classify_tier(&report), Tier::EdgeCompact);
        let rec = recommend(&report);
        assert!(rec.candidates.iter().all(|c| c.total_params_b <= 2.0));
        assert!(
            rec.candidates
                .iter()
                .all(|c| c.quant == "Q4_K_M" || c.quant == "IQ4_XS")
        );
        assert!(rec.candidates.iter().all(|c| c.est_ram_mb <= 2000));
    }

    #[test]
    fn small_vram_gpu_falls_back() {
        let report = hw(16384, 10000, vec![gpu(4096, "vulkan")], "vulkan");
        assert_eq!(classify_tier(&report), Tier::MoeSparse);
    }

    #[test]
    fn size_estimates_plausible() {
        let q4 = estimate_size_bytes(7.6, "Q4_K_M").unwrap();
        assert!(
            (4_400_000_000..5_000_000_000).contains(&q4),
            "7B Q4_K_M ~4.6GB, got {q4}"
        );
        let q8 = estimate_size_bytes(7.6, "Q8_0").unwrap();
        assert!(q8 > q4);
        assert!(estimate_size_bytes(1.5, "NOT_A_QUANT").is_none());
    }

    #[test]
    fn download_urls_are_hf() {
        let report = hw(16384, 10000, vec![], "cpu");
        for c in recommend(&report).candidates {
            assert!(c.download_url.starts_with("https://huggingface.co/"));
            assert!(c.filename.ends_with(".gguf"));
        }
    }
}
