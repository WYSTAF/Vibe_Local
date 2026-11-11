# Vibe Local

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS%20%7C%20Android-lightgrey.svg)](#)

**Zero-cloud local AI inference manager** with a hardware-aware **MoE vs. Dense** recommender, a **10-pillar independent GGUF quantization benchmark suite**, and a native **Android + PC** ecosystem node that plugs straight into **Vibe Router** (`127.0.0.1:20128`) and **Vibe Assembly**.

Vibe Local runs GGUF models fully offline on your own hardware, tells you *which architecture and quantization actually fit your device*, measures *what you lose* when you quantize, and exposes an OpenAI-compatible endpoint that Vibe Router can route to at **zero cost** (`X-Vibe-Router-Cost-Micros: 0`).

---

## Why MoE vs. Dense matters on your hardware

| Your device | Tier | What Vibe Local recommends | Why |
|---|---|---|---|
| **Nothing Phone 2a** (12GB LPDDR5, CPU/NEON) | `moe_sparse` | OLMoE-1B-7B (1.3B active), Granite-3.1-3B-A800M (0.8B active) | Total weights fit in RAM; only 0.8–1.3B params activate per token → 2–3× faster decode, less heat |
| **PC, i5-4690 + 16GB DDR3, CPU-only** | `moe_sparse` | Same sparse MoE family | DDR3 bandwidth is the bottleneck; sparse activation moves far fewer bytes per token |
| **AMD RX 580 8GB (Vulkan)** / CUDA GPU ≥6GB | `gpu_dense` | Qwen2.5-Coder-7B, DeepSeek-R1-Distill-Qwen-7B, Llama-3.1-8B at `Q4_K_M`/`Q5_K_M` (4.6–5.4GB) | 100% of weights fit in fast VRAM with `-ngl 99` |
| **<2.5GB free RAM** (budget phones, small VMs) | `edge_compact` | Qwen2.5-Coder-1.5B, DeepSeek-R1-Distill-Qwen-1.5B, SmolLM2-1.7B at `Q4_K_M`/`IQ4_XS` | Prevents Android LMKD kills and desktop OOM |

All hardware numbers are **read from the OS** — `GlobalMemoryStatusEx` (Windows), `/proc/meminfo` (Linux/Android), `sysctl hw.memsize` (macOS), real CPU feature probes (`AVX2`, `AVX512`, `NEON`, `dotprod`, `i8mm`), and GPU/VRAM detection via the Windows registry (`HardwareInformation.qwMemorySize`), `/sys/class/drm` + `dlopen` (Linux), `system_profiler` (macOS). Nothing is hardcoded.

## Quick start (PC)

```bash
cargo build --release

# 1. See your hardware + the MoE-vs-Dense verdict
./target/release/vibe-local scan

# 2. Discover GGUF quants from trusted quantizers (bartowski, unsloth, ggml-org, MaziyarPanahi)
./target/release/vibe-local discover "Qwen2.5-Coder-7B"

# 3. Inspect any GGUF: architecture, quant, exact RAM needs at 2k/4k/8k/16k context
./target/release/vibe-local inspect-gguf ./models/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf

# 4. Run the 15-second VibeMark speed benchmark (downloads the model if given an HF id)
./target/release/vibe-local bench Qwen2.5-Coder-7B --json

# 5. Start the Vibe-Link server + web leaderboard on 127.0.0.1:20129
./target/release/vibe-local serve

# 6. LAN Pocket Worker mode (token-authenticated, discoverable by Vibe Router)
./target/release/vibe-local serve --lan
```

Open **http://127.0.0.1:20129/leaderboard** for the interactive dashboard: Power-per-Parameter Pareto scatter, quant degradation curves (Q8_0 → Q2_K), hardware profile switcher and the sortable 10-pillar matrix.

For full LLM inference, install [llama.cpp](https://github.com/ggml-org/llama.cpp) and put `llama-server` on `PATH` (or set `VIBE_LLAMA_SERVER`). Without it, Vibe Local runs its **deterministic built-in micro-eval engine** — a real extractive/arithmetic reasoner used for pipeline validation, clearly labeled `engine: "builtin-microeval"` in every response and benchmark. It is never presented as LLM-quality output.

## Vibe Router integration

```bash
curl http://127.0.0.1:20129/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"Hello"}],"strip_think":true}'
```

Every `/v1/*` response carries:

- `X-Vibe-Router-Cost-Micros: 0` — local inference is free
- `X-Vibe-Local-Tok-Per-Sec` — measured decode speed
- `X-Vibe-Local-Backend` — `vulkan` / `cuda` / `cpu`

Discovery: Vibe Router finds Vibe Local automatically via **mDNS** (`_vibe-local._tcp.local`) or a UDP probe (`VIBE_DISCOVER_V1` → port **20131**). In `--lan` mode every `/api/*` and `/v1/*` route requires the pre-shared `X-Vibe-Local-Token` (or `Authorization: Bearer`) header, and state-mutating requests additionally require `Content-Type: application/json` or `X-Vibe-Admin: 1`.

## Android (Pocket Worker)

The native Kotlin + Jetpack Compose app lives in [`android/`](android/README.md): Obsidian Dark Material 3, four tabs (**Scan & Match**, **10-Pillar Leaderboard**, **VibeMark**, **Chat & Vibe-Link LAN Worker**) and four safety guards:

1. **LmkdMemoryGuard** — blocks loads that would trigger low-memory kills (`ActivityManager.MemoryInfo`, 512MB margin, `onTrimMemory` auto-unload)
2. **ThermalGovernor** — `PowerManager` thermal status + native governor: **42°C throttle, 45°C pause**
3. **VibeLinkForegroundService** — keeps the LAN worker alive with a notification + wake lock
4. **AutoUnloadIdleTimer** — unloads the model after 300s idle (configurable)

Build the native bridge with `cargo ndk` (see `android/README.md`). The phone becomes a token-authenticated LAN inference node that Vibe Router can dispatch to.

## The 10 independent benchmark pillars

| # | Pillar | Measures |
|---|---|---|
| 1 | `code_generation` | HumanEval+/LiveCodeBench-style pass@1 (executed unit tests) |
| 2 | `systems_polyglot` | MultiPL-E Rust/C++/TS translation |
| 3 | `math_reasoning` | GSM8K/MATH numeric accuracy |
| 4 | `hard_science_gpqa` | GPQA Diamond multiple choice |
| 5 | `instruction_schema` | IFEval constraints + strict JSON + **FNV-1a anchor** compliance |
| 6 | `long_context_needle` | Needle-in-a-haystack retrieval at 4k/8k/16k, depths 0.1/0.5/0.9 |
| 7 | `truthfulness_anti_sycophancy` | TruthfulQA + sycophancy probes |
| 8 | `quant_retention_kl` | KL divergence & perplexity delta vs `Q8_0`/FP16 baseline |
| 9 | `hardware_throughput` | Prefill & decode tok/s on RX 580 Vulkan / i5-4690 DDR3 / Phone 2a / Tesla T4 |
| 10 | `latency_thermal_efficiency` | TTFT ms, peak RAM/VRAM MB, thermal delta °C |

Derived metrics: **`vibe_quality_score`** (weighted mean of pillars 1–8) and **`power_per_parameter = vibe_quality_score / active_params_b`** — the Pareto frontier of quality per active parameter is what separates a great MoE from a mediocre dense model on constrained hardware.

### Kaggle / local runner

```bash
# Offline unit checks (no network, no GPU)
python benchmarks/kaggle_10pillar_bench.py --self-test

# Full run on Kaggle (2x Tesla T4) or any machine with llama.cpp
python benchmarks/kaggle_10pillar_bench.py \
  --model bartowski/Qwen2.5-Coder-7B-Instruct-GGUF --quant Q4_K_M \
  --profile tesla_t4 --backend cuda \
  --out results.json --db vibe_local.db \
  --submit http://127.0.0.1:20129
```

**Provenance policy:** `benchmarks/vibe_benchmark_catalog.json` ships with 70 **seed entries explicitly labeled `"provenance": "seed_estimate"`** — illustrative estimates so the dashboard renders before you run anything. They are *not* measurements and carry no fabricated citations. Real runs produce `"provenance": "measured"` entries that supersede seeds for the same model/quant/profile key.

## Security model

- **Loopback by default** (`127.0.0.1:20129`); LAN mode is opt-in and token-gated (constant-time comparison, CSPRNG-generated 32-hex tokens).
- **SSRF-guarded downloads**: only `https://huggingface.co/` and verified HF CDN hosts; every URL is DNS-resolved and rejected if any address is loopback, link-local (`169.254.0.0/16`), RFC1918, ULA, CGNAT or multicast; redirects re-validated per hop (max 5).
- **Strict CSP** on all served assets; the leaderboard uses safe DOM construction only (`textContent`/`createElement`, zero `innerHTML`).
- **Bounded resources**: 8MB request bodies, 64MB GGUF header budget, bounded engine command channel (16), 4 HTTP workers, 16GB download ceiling, idle auto-unload.
- **Thermal safety**: inference pauses at ≥45°C and resumes below 43°C (hysteresis); throttles at ≥42°C. No sensor → honest `no_sensor` state, never fake temperatures.

## Repository layout

```
crates/vibe-local-core      Hardware probe, recommender, HF crawler, GGUF parser, catalog math, SQLite
crates/vibe-local-engine    llama-server supervisor, micro-eval engine, thermal governor, VibeMark
crates/vibe-local-cli       vibe-local binary + Vibe-Link HTTP server + UDP/mDNS discovery
crates/vibe-local-jni       Android JNI + C-ABI bridge (cdylib/staticlib)
android/                    Kotlin + Compose Pocket Worker app (4 tabs, 4 safety guards)
benchmarks/                 Kaggle 10-pillar runner + seed catalog (stdlib-only Python)
leaderboard-web/            Vanilla-JS dashboard (Pareto scatter, degradation curves, matrix)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data flow and SQLite schema.

## Verification gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
python benchmarks/kaggle_10pillar_bench.py --self-test
```

## License

Apache-2.0 — see [LICENSE](LICENSE).