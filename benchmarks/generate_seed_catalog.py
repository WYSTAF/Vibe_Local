#!/usr/bin/env python3
"""
generate_seed_catalog.py — produces vibe_benchmark_catalog.json (SEED ESTIMATES ONLY).

==============================================================================
PROVENANCE WARNING — READ BEFORE USING ANY NUMBER IN THE OUTPUT FILE
==============================================================================
This script does NOT measure anything. It fabricates *illustrative estimates*
from public quality trends (quantization retention behavior commonly reported
for GGUF k-quants) and simple memory-bandwidth reasoning per hardware profile.
Every entry it writes is labeled "provenance": "seed_estimate". These values
exist only so the leaderboard renders before real runs exist. They are
replaced by "provenance": "measured" entries when kaggle_10pillar_bench.py
executes against a real llama-server. No citations, DOIs or paper claims are
made anywhere. source_url fields point at Hugging Face repo pages only.

Run once:  python benchmarks/generate_seed_catalog.py
Stdlib only. Deterministic (seeded RNG), safe to re-run.
==============================================================================
"""

import json
import random
from pathlib import Path

GENERATED = "2026-11-12"
SCHEMA_VERSION = 1

PILLARS = [
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
]

PILLAR_LABELS = {
    "code_generation": "Code Generation (HumanEval-style subset)",
    "systems_polyglot": "Systems & Polyglot (MultiPL-E-style subset)",
    "math_reasoning": "Math Reasoning (GSM8K-style subset)",
    "hard_science_gpqa": "Hard Science (GPQA-Diamond-style subset)",
    "instruction_schema": "Instruction & Schema (IFEval-style + FNV-1a anchors)",
    "long_context_needle": "Long Context Needle Retrieval",
    "truthfulness_anti_sycophancy": "Truthfulness & Anti-Sycophancy",
    "quant_retention_kl": "Quantization Retention (KL / perplexity)",
    "hardware_throughput": "Hardware Throughput (VibeMark)",
    "latency_thermal_efficiency": "Latency & Thermal Efficiency",
}

# Weights for vibe_quality_score over the FIRST 8 pillars (sum == 1.0).
WEIGHTS = [0.16, 0.12, 0.14, 0.10, 0.12, 0.12, 0.12, 0.12]

HARDWARE_PROFILES = {
    "rx580_vulkan": {
        "label": "AMD Radeon RX 580 8GB (Vulkan)",
        "backend": "vulkan", "vram_mb": 8192, "ram_mb": 16384,
    },
    "nothing_phone_2a": {
        "label": "Nothing Phone 2a 12GB (CPU NEON)",
        "backend": "cpu", "vram_mb": 0, "ram_mb": 12288,
    },
    "i5_4690_ddr3": {
        "label": "Intel i5-4690 16GB DDR3 (CPU AVX2)",
        "backend": "cpu", "vram_mb": 0, "ram_mb": 16384,
    },
    "tesla_t4": {
        "label": "NVIDIA Tesla T4 16GB (CUDA)",
        "backend": "cuda", "vram_mb": 16384, "ram_mb": 30720,
    },
}

# Bits-per-weight approximations for GGUF quants (estimates, incl. metadata).
QUANT_BPW = {
    "Q2_K": 2.63, "Q3_K_M": 3.91, "IQ4_XS": 4.25, "IQ4_NL": 4.56,
    "Q4_K_M": 4.85, "Q5_K_M": 5.69, "Q6_K": 6.59, "Q8_0": 8.50,
}

# Baseline retention of FP16 quality per quant (public trend estimates).
QUANT_RETENTION = {
    "Q8_0": 0.990, "Q6_K": 0.985, "Q5_K_M": 0.980, "Q4_K_M": 0.965,
    "IQ4_NL": 0.955, "IQ4_XS": 0.950, "Q3_K_M": 0.900, "Q2_K": 0.780,
}

# Illustrative KL divergence vs Q8_0 baseline (nats, first-token top-k dist).
QUANT_KL = {
    "Q8_0": 0.002, "Q6_K": 0.006, "Q5_K_M": 0.010, "Q4_K_M": 0.020,
    "IQ4_NL": 0.026, "IQ4_XS": 0.030, "Q3_K_M": 0.080, "Q2_K": 0.250,
}

# Effective memory bandwidth (GB/s) available to llama.cpp per profile.
# Calibrated so that: RX580 Vulkan 7B Q4_K_M decode ~25-35 tok/s,
# i5-4690 DDR3 7B Q4_K_M ~4-7 tok/s, Phone 2a 1.5B Q4_K_M ~12-20 tok/s,
# Tesla T4 7B Q4_K_M ~45-60 tok/s. These are estimates, not measurements.
PROFILE_BANDWIDTH_GBS = {
    "rx580_vulkan": 140.0, "i5_4690_ddr3": 25.0,
    "nothing_phone_2a": 14.0, "tesla_t4": 260.0,
}
PREFILL_FACTOR = {"cuda": 12.0, "vulkan": 9.0, "cpu": 4.5}
TTFT_OVERHEAD_MS = {"cuda": 60.0, "vulkan": 90.0, "cpu": 150.0}
MOE_EFFICIENCY = {"cpu": 0.45, "vulkan": 0.50, "cuda": 0.60}
THERMAL_BASE_C = {"tesla_t4": 12.0, "rx580_vulkan": 15.0,
                  "i5_4690_ddr3": 18.0, "nothing_phone_2a": 8.0}

# model_id -> metadata. base_quality = illustrative FP16-reference scores for
# the 7 quality pillars (code, systems, math, gpqa, instruct, needle, truth).
MODELS = {
    "Qwen2.5-Coder-1.5B-Instruct": {
        "architecture": "qwen2", "family": "dense",
        "total_params_b": 1.54, "active_params_b": 1.54, "context_length": 32768,
        "base_quality": [0.55, 0.48, 0.45, 0.24, 0.62, 0.86, 0.52],
        "repos": {
            "bartowski": "bartowski/Qwen2.5-Coder-1.5B-Instruct-GGUF",
            "unsloth": "unsloth/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        },
    },
    "Qwen2.5-Coder-3B-Instruct": {
        "architecture": "qwen2", "family": "dense",
        "total_params_b": 3.09, "active_params_b": 3.09, "context_length": 32768,
        "base_quality": [0.68, 0.60, 0.58, 0.27, 0.70, 0.89, 0.56],
        "repos": {
            "bartowski": "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF",
            "unsloth": "unsloth/Qwen2.5-Coder-3B-Instruct-GGUF",
        },
    },
    "Qwen2.5-Coder-7B-Instruct": {
        "architecture": "qwen2", "family": "dense",
        "total_params_b": 7.62, "active_params_b": 7.62, "context_length": 32768,
        "base_quality": [0.78, 0.70, 0.70, 0.31, 0.78, 0.92, 0.60],
        "repos": {
            "bartowski": "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
            "unsloth": "unsloth/Qwen2.5-Coder-7B-Instruct-GGUF",
        },
    },
    "DeepSeek-R1-Distill-Qwen-1.5B": {
        "architecture": "qwen2", "family": "dense",
        "total_params_b": 1.78, "active_params_b": 1.78, "context_length": 131072,
        "base_quality": [0.46, 0.40, 0.56, 0.25, 0.48, 0.84, 0.50],
        "repos": {
            "bartowski": "bartowski/DeepSeek-R1-Distill-Qwen-1.5B-GGUF",
            "unsloth": "unsloth/DeepSeek-R1-Distill-Qwen-1.5B-GGUF",
        },
    },
    "DeepSeek-R1-Distill-Qwen-7B": {
        "architecture": "qwen2", "family": "dense",
        "total_params_b": 7.62, "active_params_b": 7.62, "context_length": 131072,
        "base_quality": [0.66, 0.58, 0.80, 0.36, 0.58, 0.90, 0.55],
        "repos": {
            "bartowski": "bartowski/DeepSeek-R1-Distill-Qwen-7B-GGUF",
            "unsloth": "unsloth/DeepSeek-R1-Distill-Qwen-7B-GGUF",
        },
    },
    "OLMoE-1B-7B-0924-Instruct": {
        "architecture": "olmoe", "family": "moe",
        "total_params_b": 6.9, "active_params_b": 1.3, "context_length": 4096,
        "base_quality": [0.31, 0.28, 0.26, 0.25, 0.44, 0.80, 0.48],
        "repos": {"bartowski": "bartowski/OLMoE-1B-7B-0924-Instruct-GGUF"},
    },
    "granite-3.1-3b-a800m-instruct": {
        "architecture": "granitemoe", "family": "moe",
        "total_params_b": 3.3, "active_params_b": 0.8, "context_length": 128000,
        "base_quality": [0.43, 0.38, 0.36, 0.26, 0.56, 0.83, 0.52],
        "repos": {"ibm-granite": "ibm-granite/granite-3.1-3b-a800m-instruct-GGUF"},
    },
    "Llama-3.2-3B-Instruct": {
        "architecture": "llama", "family": "dense",
        "total_params_b": 3.21, "active_params_b": 3.21, "context_length": 131072,
        "base_quality": [0.49, 0.44, 0.43, 0.27, 0.64, 0.88, 0.55],
        "repos": {
            "bartowski": "bartowski/Llama-3.2-3B-Instruct-GGUF",
            "unsloth": "unsloth/Llama-3.2-3B-Instruct-GGUF",
        },
    },
    "SmolLM2-1.7B-Instruct": {
        "architecture": "llama", "family": "dense",
        "total_params_b": 1.71, "active_params_b": 1.71, "context_length": 8192,
        "base_quality": [0.29, 0.25, 0.23, 0.23, 0.46, 0.62, 0.45],
        "repos": {
            "bartowski": "bartowski/SmolLM2-1.7B-Instruct-GGUF",
            "unsloth": "unsloth/SmolLM2-1.7B-Instruct-GGUF",
        },
    },
    "Meta-Llama-3.1-8B-Instruct": {
        "architecture": "llama", "family": "dense",
        "total_params_b": 8.03, "active_params_b": 8.03, "context_length": 131072,
        "base_quality": [0.61, 0.55, 0.51, 0.32, 0.72, 0.93, 0.58],
        "repos": {
            "bartowski": "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
            "unsloth": "unsloth/Meta-Llama-3.1-8B-Instruct-GGUF",
        },
    },
    "Qwen1.5-MoE-A2.7B-Chat": {
        "architecture": "qwen2moe", "family": "moe",
        "total_params_b": 14.3, "active_params_b": 2.7, "context_length": 8192,
        "base_quality": [0.41, 0.36, 0.36, 0.26, 0.52, 0.66, 0.50],
        "repos": {
            "bartowski": "bartowski/Qwen1.5-MoE-A2.7B-Chat-GGUF",
            "qwen": "Qwen/Qwen1.5-MoE-A2.7B-Chat-GGUF",
        },
    },
}

# (model_id, quantizer, quant, hardware_profile) — sensible model/hardware fit.
PLAN = [
    # --- Nothing Phone 2a (small models) ---
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "Q2_K", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "Q3_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "IQ4_XS", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "Q4_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "Q5_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "bartowski", "Q8_0", "nothing_phone_2a"),
    ("Qwen2.5-Coder-1.5B-Instruct", "unsloth", "Q4_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-3B-Instruct", "bartowski", "Q3_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-3B-Instruct", "bartowski", "Q4_K_M", "nothing_phone_2a"),
    ("Qwen2.5-Coder-3B-Instruct", "unsloth", "Q4_K_M", "nothing_phone_2a"),
    ("DeepSeek-R1-Distill-Qwen-1.5B", "unsloth", "Q4_K_M", "nothing_phone_2a"),
    ("DeepSeek-R1-Distill-Qwen-1.5B", "unsloth", "Q5_K_M", "nothing_phone_2a"),
    ("granite-3.1-3b-a800m-instruct", "ibm-granite", "Q4_K_M", "nothing_phone_2a"),
    ("Llama-3.2-3B-Instruct", "unsloth", "Q4_K_M", "nothing_phone_2a"),
    ("SmolLM2-1.7B-Instruct", "bartowski", "Q4_K_M", "nothing_phone_2a"),
    ("SmolLM2-1.7B-Instruct", "bartowski", "Q8_0", "nothing_phone_2a"),
    # --- i5-4690 DDR3 CPU (small dense + MoE with small active set) ---
    ("Qwen2.5-Coder-1.5B-Instruct", "unsloth", "Q8_0", "i5_4690_ddr3"),
    ("Qwen2.5-Coder-3B-Instruct", "bartowski", "Q5_K_M", "i5_4690_ddr3"),
    ("Qwen2.5-Coder-3B-Instruct", "bartowski", "Q8_0", "i5_4690_ddr3"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q4_K_M", "i5_4690_ddr3"),
    ("DeepSeek-R1-Distill-Qwen-1.5B", "bartowski", "Q4_K_M", "i5_4690_ddr3"),
    ("DeepSeek-R1-Distill-Qwen-1.5B", "bartowski", "Q8_0", "i5_4690_ddr3"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q2_K", "i5_4690_ddr3"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q3_K_M", "i5_4690_ddr3"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q4_K_M", "i5_4690_ddr3"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q5_K_M", "i5_4690_ddr3"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q8_0", "i5_4690_ddr3"),
    ("granite-3.1-3b-a800m-instruct", "ibm-granite", "Q4_K_M", "i5_4690_ddr3"),
    ("granite-3.1-3b-a800m-instruct", "ibm-granite", "Q8_0", "i5_4690_ddr3"),
    ("SmolLM2-1.7B-Instruct", "bartowski", "Q2_K", "i5_4690_ddr3"),
    ("SmolLM2-1.7B-Instruct", "unsloth", "Q4_K_M", "i5_4690_ddr3"),
    ("Qwen1.5-MoE-A2.7B-Chat", "bartowski", "Q3_K_M", "i5_4690_ddr3"),
    ("Qwen1.5-MoE-A2.7B-Chat", "bartowski", "Q4_K_M", "i5_4690_ddr3"),
    ("Qwen1.5-MoE-A2.7B-Chat", "bartowski", "Q5_K_M", "i5_4690_ddr3"),
    # --- RX 580 8GB Vulkan (<= ~5.7GB models fully offloaded) ---
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q2_K", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q3_K_M", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "IQ4_XS", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "IQ4_NL", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q4_K_M", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q5_K_M", "rx580_vulkan"),
    ("Qwen2.5-Coder-7B-Instruct", "unsloth", "Q4_K_M", "rx580_vulkan"),
    ("DeepSeek-R1-Distill-Qwen-7B", "bartowski", "Q4_K_M", "rx580_vulkan"),
    ("OLMoE-1B-7B-0924-Instruct", "bartowski", "Q4_K_M", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q2_K", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q3_K_M", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q4_K_M", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q5_K_M", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q8_0", "rx580_vulkan"),
    ("Llama-3.2-3B-Instruct", "unsloth", "Q4_K_M", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "Q2_K", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "Q3_K_M", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "IQ4_XS", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "Q4_K_M", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "Q5_K_M", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "unsloth", "Q4_K_M", "rx580_vulkan"),
    ("Meta-Llama-3.1-8B-Instruct", "unsloth", "IQ4_NL", "rx580_vulkan"),
    # --- Tesla T4 16GB CUDA (7B/8B incl. Q8_0, MoE) ---
    ("Qwen2.5-Coder-7B-Instruct", "bartowski", "Q8_0", "tesla_t4"),
    ("Qwen2.5-Coder-7B-Instruct", "unsloth", "Q4_K_M", "tesla_t4"),
    ("Qwen2.5-Coder-7B-Instruct", "unsloth", "IQ4_XS", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "bartowski", "Q3_K_M", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "bartowski", "Q4_K_M", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "bartowski", "Q5_K_M", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "bartowski", "Q8_0", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "unsloth", "Q4_K_M", "tesla_t4"),
    ("DeepSeek-R1-Distill-Qwen-7B", "unsloth", "IQ4_NL", "tesla_t4"),
    ("Llama-3.2-3B-Instruct", "bartowski", "Q4_K_M", "tesla_t4"),
    ("Meta-Llama-3.1-8B-Instruct", "bartowski", "Q8_0", "tesla_t4"),
    ("Meta-Llama-3.1-8B-Instruct", "unsloth", "Q4_K_M", "tesla_t4"),
    ("Qwen1.5-MoE-A2.7B-Chat", "qwen", "Q4_K_M", "tesla_t4"),
    ("Qwen1.5-MoE-A2.7B-Chat", "qwen", "Q5_K_M", "tesla_t4"),
]

PILLAR_N = {
    "code_generation": 6, "systems_polyglot": 5, "math_reasoning": 6,
    "hard_science_gpqa": 5, "instruction_schema": 6, "long_context_needle": 9,
    "truthfulness_anti_sycophancy": 6, "quant_retention_kl": 8,
    "hardware_throughput": 1, "latency_thermal_efficiency": 1,
}
PILLAR_METRIC = {
    "code_generation": "pass@1", "systems_polyglot": "accuracy",
    "math_reasoning": "exact_match", "hard_science_gpqa": "accuracy",
    "instruction_schema": "constraint_compliance", "long_context_needle": "retrieval_accuracy",
    "truthfulness_anti_sycophancy": "accuracy", "quant_retention_kl": "retention_score",
    "hardware_throughput": "decode_tps_normalized", "latency_thermal_efficiency": "composite",
}
# Per-pillar sensitivity to quantization damage (reasoning pillars hurt more).
QUALITY_SENSITIVITY = [1.15, 1.10, 1.20, 1.00, 1.05, 0.85, 0.90]


def clamp(x, lo=0.0, hi=1.0):
    return max(lo, min(hi, x))


def retention_for(model, quant):
    """Adjusted retention: small models and MoE degrade faster at low quants."""
    r = QUANT_RETENTION[quant]
    drop = 1.0 - r
    active = model["active_params_b"]
    if active < 7.0:
        drop += drop * 0.25 * (7.0 - active) / 7.0
    if model["family"] == "moe" and r < 0.96:
        drop += (1.0 - r) * 0.25
    return clamp(1.0 - drop, 0.40, 0.999)


def size_bytes_for(model, quant):
    return int(round(model["total_params_b"] * 1e9 * QUANT_BPW[quant] / 8.0))


def throughput_estimate(model, quant, profile, rng):
    prof = HARDWARE_PROFILES[profile]
    backend = prof["backend"]
    bpw = QUANT_BPW[quant]
    size_b = size_bytes_for(model, quant)
    size_mb = size_b / (1024 * 1024)
    active_bytes_gb = model["active_params_b"] * bpw / 8.0
    overhead_gb = 0.15 if model["family"] == "moe" else 0.02
    decode = PROFILE_BANDWIDTH_GBS[profile] / (active_bytes_gb + overhead_gb)
    if model["family"] == "moe":
        decode *= MOE_EFFICIENCY[backend]
    decode *= rng.uniform(0.94, 1.06)
    decode = round(decode, 1)
    prefill = round(decode * PREFILL_FACTOR[backend] * rng.uniform(0.9, 1.1), 1)
    ttft = round(64.0 / max(prefill, 0.1) * 1000.0 + TTFT_OVERHEAD_MS[backend]
                 + rng.uniform(-20, 40), 1)
    kv_mb = round(40 + 60 * model["total_params_b"])
    if backend == "cpu":
        peak_ram = round(size_mb + kv_mb + 700)
        peak_vram = 0
    else:
        peak_ram = round(900 + size_mb * 0.15)
        peak_vram = min(prof["vram_mb"], round(size_mb + kv_mb + 250))
    thermal = round(THERMAL_BASE_C[profile] + rng.uniform(-3, 3), 1)
    return {
        "prefill_tps": prefill, "decode_tps": decode, "ttft_ms": ttft,
        "peak_ram_mb": peak_ram, "peak_vram_mb": peak_vram,
        "thermal_delta_c": thermal,
    }


def latency_score(ttft_ms, thermal_delta_c):
    """Same composite used by the runner: TTFT <= 500ms is great; thermal
    delta <= 10C is great. 0.6 * ttft_component + 0.4 * thermal_component."""
    ttft_c = 1.0 if ttft_ms <= 500 else max(0.0, 1.0 - (ttft_ms - 500) / 2500.0)
    if thermal_delta_c is None:
        return round(ttft_c, 4)
    th_c = 1.0 if thermal_delta_c <= 10 else max(0.0, 1.0 - (thermal_delta_c - 10) / 25.0)
    return round(0.6 * ttft_c + 0.4 * th_c, 4)


def filename_for(model_id, quant, quantizer):
    if quantizer == "qwen":
        return f"qwen1_5-moe-a2_7b-chat-{quant.lower()}.gguf"
    return f"{model_id}-{quant}.gguf"


def build_entry(model_id, quantizer, quant, profile):
    model = MODELS[model_id]
    repo = model["repos"][quantizer]
    rng = random.Random(f"{model_id}|{repo}|{quant}|{profile}")
    fname = filename_for(model_id, quant, quantizer)
    ret = retention_for(model, quant)
    pillars = {}
    for i, pid in enumerate(PILLARS[:7]):
        base = model["base_quality"][i]
        drop = (1.0 - ret) * QUALITY_SENSITIVITY[i]
        score = clamp(base * (1.0 - drop) + rng.uniform(-0.008, 0.008), 0.02, 0.99)
        pillars[pid] = {
            "score": round(score, 4),
            "metric": PILLAR_METRIC[pid],
            "n": PILLAR_N[pid],
            "detail": {"subset": PILLAR_LABELS[pid], "estimate": True},
        }
    kl_scale = (1.0 - ret) / max(1e-9, (1.0 - QUANT_RETENTION[quant]))
    kl = round(QUANT_KL[quant] * kl_scale * rng.uniform(0.9, 1.1), 5)
    pillars["quant_retention_kl"] = {
        "score": round(clamp(ret + rng.uniform(-0.004, 0.004), 0.02, 0.999), 4),
        "metric": PILLAR_METRIC["quant_retention_kl"],
        "n": PILLAR_N["quant_retention_kl"],
        "detail": {
            "kl_divergence": kl,
            "perplexity_delta": round(kl * 40.0, 3),
            "baseline_quant": "Q8_0",
            "method": "seed_estimate",
            "estimate": True,
        },
    }
    tp = throughput_estimate(model, quant, profile, rng)
    pillars["hardware_throughput"] = {
        "score": round(min(1.0, tp["decode_tps"] / 60.0), 4),
        "metric": PILLAR_METRIC["hardware_throughput"],
        "n": 1,
        "detail": {"decode_tps": tp["decode_tps"], "prefill_tps": tp["prefill_tps"],
                   "budget_secs": 15, "estimate": True},
    }
    pillars["latency_thermal_efficiency"] = {
        "score": latency_score(tp["ttft_ms"], tp["thermal_delta_c"]),
        "metric": PILLAR_METRIC["latency_thermal_efficiency"],
        "n": 1,
        "detail": {"ttft_ms": tp["ttft_ms"], "thermal_delta_c": tp["thermal_delta_c"],
                   "peak_ram_mb": tp["peak_ram_mb"], "peak_vram_mb": tp["peak_vram_mb"],
                   "estimate": True},
    }
    vibe = round(sum(w * pillars[p]["score"] for w, p in zip(WEIGHTS, PILLARS[:8])), 6)
    return {
        "model_id": model_id,
        "hf_repo": repo,
        "filename": fname,
        "download_url": f"https://huggingface.co/{repo}/resolve/main/{fname}",
        "quant": quant,
        "architecture": model["architecture"],
        "family": model["family"],
        "total_params_b": model["total_params_b"],
        "active_params_b": model["active_params_b"],
        "size_bytes": size_bytes_for(model, quant),
        "context_length": model["context_length"],
        "hardware_profile": profile,
        "provenance": "seed_estimate",
        "source_url": f"https://huggingface.co/{repo}",
        "pillars": pillars,
        "throughput": tp,
        "vibe_quality_score": vibe,
        "power_per_parameter": round(vibe / model["active_params_b"], 6),
    }


def main():
    entries = [build_entry(*p) for p in PLAN]
    catalog = {
        "schema_version": SCHEMA_VERSION,
        "generated": GENERATED,
        "provenance_note": (
            "SEED DATASET — ILLUSTRATIVE ESTIMATES ONLY. Every value in this catalog is an "
            "illustrative estimate derived from public quality trends for GGUF quantization "
            "and simple memory-bandwidth reasoning per hardware profile. These are NOT "
            "measured results, and no citations, DOIs or paper claims are made. All entries "
            "carry provenance='seed_estimate' and exist only so the leaderboard renders "
            "before real runs exist. They are replaced by provenance='measured' entries "
            "produced by kaggle_10pillar_bench.py when the runner executes against a real "
            "llama-server. source_url fields point to Hugging Face repo pages."
        ),
        "pillars": PILLARS,
        "pillar_labels": PILLAR_LABELS,
        "hardware_profiles": HARDWARE_PROFILES,
        "entries": entries,
    }
    out = Path(__file__).resolve().parent / "vibe_benchmark_catalog.json"
    out.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    by_profile = {}
    for e in entries:
        by_profile[e["hardware_profile"]] = by_profile.get(e["hardware_profile"], 0) + 1
    print(f"Wrote {out} with {len(entries)} seed-estimate entries.")
    for k, v in sorted(by_profile.items()):
        print(f"  {k}: {v}")
    print("REMINDER: all values are illustrative estimates, NOT measurements.")


if __name__ == "__main__":
    main()