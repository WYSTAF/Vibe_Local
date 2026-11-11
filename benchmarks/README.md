# Vibe Local — 10-Pillar GGUF Benchmark Suite

Turnkey benchmark runner + seed leaderboard catalog for local GGUF models.
**Python stdlib only** (no pip installs) — runs on Kaggle (2x Tesla T4) and
locally (RX 580 Vulkan, i5-4690 DDR3 CPU, Nothing Phone 2a via Termux, ...).
Tested with Python 3.14 on Windows; any Python >= 3.10 works.

## Files

| File | Purpose |
|---|---|
| `kaggle_10pillar_bench.py` | The runner: HF resolver, llama-server orchestration, 10 pillar scorers, JSON/SQLite/HTTP output, offline `--self-test`. |
| `vibe_benchmark_catalog.json` | **Seed dataset** (70 entries, `schema_version: 1`) so the leaderboard renders before real runs exist. |
| `generate_seed_catalog.py` | Deterministic generator for the seed catalog. **Produces illustrative estimates, not measurements.** |
| `README.md` | This file. |

## Provenance policy (important)

- Every catalog entry is labeled `"provenance": "seed_estimate"`. The values
  are **illustrative estimates** derived from public quantization-quality
  trends and memory-bandwidth reasoning per hardware profile. They are **NOT
  measured results**, and no citations, DOIs, or paper claims are made.
  `source_url` fields point only at Hugging Face repo pages.
- When the runner executes against a real `llama-server`, it emits entries
  labeled `"provenance": "measured"`, which replace the seed estimates on the
  leaderboard.
- If `llama-server` is missing, the runner exits with build instructions.
  It **never fabricates results**. Unavailable measurements (e.g. thermal
  sensors on Windows) are reported as `null`, and pillars that cannot run
  honestly are skipped (the vibe score renormalizes over available pillars).

## Quick start

```bash
# Offline unit checks (no network, no server, no GPU needed) — must exit 0:
python benchmarks/kaggle_10pillar_bench.py --self-test

# Discover GGUF repos on Hugging Face (trusted quantizers:
# bartowski / unsloth / ggml-org / MaziyarPanahi):
python benchmarks/kaggle_10pillar_bench.py --list-models --model "qwen2.5 coder"
python benchmarks/kaggle_10pillar_bench.py --list-models --model bartowski/Qwen2.5-Coder-7B-Instruct-GGUF
```

You need a `llama-server` binary from [llama.cpp](https://github.com/ggml-org/llama.cpp).
The runner searches `PATH`, `./llama.cpp`, `./llama.cpp/build/bin`, and
`/kaggle/working/llama.cpp/build/bin`, or take `--llama-server <path>`.

## Kaggle (2x Tesla T4)

In a notebook (GPU accelerator enabled):

```python
!apt-get update -qq && apt-get install -y -qq cmake libcurl4-openssl-dev
!git clone --depth 1 https://github.com/ggml-org/llama.cpp
!cmake -S llama.cpp -B llama.cpp/build -DGGML_CUDA=ON
!cmake --build llama.cpp/build --config Release -j --target llama-server
```

```python
!python benchmarks/kaggle_10pillar_bench.py \
    --model bartowski/Qwen2.5-Coder-7B-Instruct-GGUF --quant Q4_K_M \
    --profile tesla_t4 --backend cuda --device 0 \
    --llama-server llama.cpp/build/bin/llama-server \
    --budget-secs 15 --out results.json --db vibe_bench.sqlite
```

- Use `--device 1` to benchmark on the second T4.
- The runner downloads the GGUF from HF (SSRF-guarded, see below) into
  `--download-dir` (default `models/`), launches `llama-server` on
  `127.0.0.1:18080` with `-ngl 99 --ctx-size 4096`, runs the pillars, and
  shuts the server down.
- The needle pillar temporarily restarts the server with a larger
  `--ctx-size` (up to ~16.9k) for the 8k/16k haystacks.
- Submit to your leaderboard: `--submit https://your-leaderboard.example.com`
  (POSTs `{"entry": {... provenance: "measured" ...}}` to
  `/api/leaderboard/submit`).

## Local

### AMD RX 580 8GB (Vulkan)

```bash
cmake -S llama.cpp -B llama.cpp/build -DGGML_VULKAN=ON
cmake --build llama.cpp/build --config Release -j --target llama-server
python benchmarks/kaggle_10pillar_bench.py \
  --model bartowski/Meta-Llama-3.1-8B-Instruct-GGUF --quant Q4_K_M \
  --profile rx580_vulkan --backend vulkan \
  --llama-server llama.cpp/build/bin/llama-server --out results.json
```

Keep models <= ~5.7 GB so they fully offload into 8 GB VRAM (Q5_K_M and below
for 7B/8B). Backend selection uses `GGML_VK_DEVICE`.

### Intel i5-4690, 16 GB DDR3 (CPU AVX2)

```bash
cmake -S llama.cpp -B llama.cpp/build
cmake --build llama.cpp/build --config Release -j --target llama-server
python benchmarks/kaggle_10pillar_bench.py \
  --model ./models/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf \
  --profile i5_4690_ddr3 --backend cpu --budget-secs 15
```

Expect ~4-7 tok/s decode on 7B Q4_K_M; MoE models with small active sets
(Granite-3.1-3B-A800M, OLMoE-1B-7B) are much faster. CPU runs are slow:
consider `--pillars code_generation,math_reasoning,hardware_throughput`
for a quick pass.

### Nothing Phone 2a (Termux, CPU NEON)

```bash
pkg install python cmake git clang
git clone --depth 1 https://github.com/ggml-org/llama.cpp
cmake -S llama.cpp -B llama.cpp/build && cmake --build llama.cpp/build -j4 --target llama-server
python benchmarks/kaggle_10pillar_bench.py \
  --model bartowski/Qwen2.5-Coder-1.5B-Instruct-GGUF --quant Q4_K_M \
  --profile nothing_phone_2a --backend cpu \
  --llama-server llama.cpp/build/bin/llama-server
```

Thermal deltas are read from `/sys/class/thermal` when the kernel exposes
them; otherwise they are `null` (never faked).

## CLI reference

| Flag | Meaning |
|---|---|
| `--self-test` | Offline unit checks (FNV-1a, KL, Pareto, needle, extractors, SSRF, catalog schema/provenance). Exits non-zero on any failure. |
| `--model` | HF repo id (`owner/name`) or local `.gguf` path. With `--list-models`: search query or repo id. |
| `--quant` | GGUF quant to select (default `Q4_K_M` for HF downloads). |
| `--profile` | `rx580_vulkan` / `nothing_phone_2a` / `i5_4690_ddr3` / `tesla_t4` (auto: `tesla_t4` if CUDA detected, else `local_cpu`). |
| `--backend` | `cuda` / `vulkan` / `cpu` (sets `CUDA_VISIBLE_DEVICES` / `GGML_VK_DEVICE`). |
| `--llama-server` | Explicit binary path. |
| `--out` | Results JSON (default `benchmark_results.json`). |
| `--db` | SQLite persistence (unified schema below). |
| `--submit` | Leaderboard base URL; POSTs the measured entry. |
| `--pillars` | Comma-separated subset of pillar ids. |
| `--budget-secs` | VibeMark throughput budget (default 15 s). |
| `--list-models` | HF discovery via `/api/models?search=...&filter=gguf&sort=downloads&limit=20`. |
| `--baseline` | Explicit Q8_0 `.gguf` for the KL retention pillar. |
| `--download-dir`, `--port`, `--device`, `--load-timeout`, `--catalog` | Download dir (default `models/`), server port (18080), GPU index (0), load timeout (300 s), seed catalog path. |

### HF download security (SSRF guard)

Only `https://huggingface.co` and HF CDN hosts (`cdn-lfs.huggingface.co`,
`cdn-lfs-us-1.huggingface.co`, `cas-bridge.xethub.hf.co`) are allowed.
Hostnames are resolved and loopback / link-local (169.254/16) / RFC1918 /
ULA / reserved addresses are rejected via `ipaddress`. Redirects are
followed **manually** (max 5 hops) and every hop is re-validated.

## The 10 pillars

Each pillar scores a small **embedded subset** (4-8 genuine-style items with
deterministic expected answers) — these are subset scores, **not** full
HumanEval/GSM8K/GPQA leaderboard numbers.

| # | Pillar | Method | Metric |
|---|---|---|---|
| 1 | `code_generation` | 6 HumanEval-style prompts; extract Python block; execute via `python -I -c` subprocess (10 s timeout) against embedded unit tests. | pass@1 |
| 2 | `systems_polyglot` | 5 MultiPL-E-style Rust/C++/TS items; compile+run when `rustc`/`g++`/`tsc` exist, else exact-signature/AST-lite heuristics (`detail.scoring="heuristic"`). | accuracy |
| 3 | `math_reasoning` | 6 GSM8K-style items; final-number extraction (`####` marker or last number), exact match. | exact_match |
| 4 | `hard_science_gpqa` | 5 GPQA-Diamond-style MCQs; letter extraction. | accuracy |
| 5 | `instruction_schema` | IFEval-style constraints (exact word count, JSON-only, keyword, lowercase) + 2 FNV-1a anchor items (output the token whose `fnv1a32` equals a given value; verified with the implemented hash). Anchors are intentionally hard — models can't compute FNV-1a mentally. | compliance |
| 6 | `long_context_needle` | Seeded haystack (seed 20261112), ~4k/8k/16k tokens x depths 0.1/0.5/0.9; secret code retrieval. Words approximate tokens 1:1. | retrieval accuracy |
| 7 | `truthfulness_anti_sycophancy` | 4 TruthfulQA-style items + 2 sycophancy probes (user asserts a wrong fact); keyword/regex grading (`detail.scoring="keyword_heuristic"`). | accuracy |
| 8 | `quant_retention_kl` | Same 8-prompt set on target quant and Q8_0 baseline (sibling file or `--baseline`): top-k KL via `/completion` `n_probs` or `/v1` logprobs (`score = exp(-KL)`); else honest `retention_proxy` vs the catalog Q8_0 seed baseline (flagged as an estimate in `detail`). | retention |
| 9 | `hardware_throughput` | 15 s VibeMark: ~512-token prefill probe + decode loop, `time.perf_counter` + server timings; peak RSS via `/proc/<pid>/status` (Linux) or ctypes `OpenProcess`/`K32GetProcessMemoryInfo` (Windows). `score = min(1, decode_tps/60)`. | normalized tok/s |
| 10 | `latency_thermal_efficiency` | Streaming TTFT; thermal delta from `/sys/class/thermal` or `nvidia-smi` before/after (`null` when unavailable); VRAM via `nvidia-smi`. `0.6*ttft_component + 0.4*thermal_component` (TTFT <= 500 ms great, delta <= 10 C great). | composite |

`vibe_quality_score` = weighted mean of pillars 1-8 with weights
`.16, .12, .14, .10, .12, .12, .12, .12` (renormalized if pillars were
skipped). `power_per_parameter = vibe_quality_score / active_params_b`.

## SQLite unified schema (shared with the Rust app)

`--db` creates/updates (WAL journal, `busy_timeout=5000`):

```sql
benchmarks(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL,
  model_id TEXT NOT NULL, hf_repo TEXT, filename TEXT, quant TEXT,
  hardware_profile TEXT, backend TEXT, pillar TEXT NOT NULL, score REAL NOT NULL,
  metrics_json TEXT NOT NULL, provenance TEXT NOT NULL DEFAULT 'measured',
  source_url TEXT);
vibemark_runs(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, model_id TEXT,
  backend TEXT, prefill_tps REAL, decode_tps REAL, ttft_ms REAL,
  peak_ram_mb REAL, duration_s REAL, provenance TEXT);
```

## Regenerating the seed catalog

```bash
python benchmarks/generate_seed_catalog.py
```

Deterministic (seeded RNG). 70 entries across the 4 hardware profiles,
covering Qwen2.5-Coder 1.5B/3B/7B, DeepSeek-R1-Distill-Qwen 1.5B/7B,
OLMoE-1B-7B, Granite-3.1-3B-A800M, Llama-3.2-3B, SmolLM2-1.7B,
Llama-3.1-8B, Qwen1.5-MoE-A2.7B (bartowski + unsloth where both exist,
plus official ibm-granite/Qwen GGUF repos). **All values are estimates.**

## Honest limitations

- Pillar datasets are tiny embedded subsets; scores are noisy and not
  comparable to full public benchmarks.
- Truthfulness and (without toolchains) polyglot scoring are heuristics.
- Needle "tokens" are word-count approximations; real tokenizers differ.
- The KL pillar needs a Q8_0 baseline file and server logprob support;
  otherwise it falls back to a clearly-flagged retention proxy.
- Thermal/VRAM readings require exposed sensors (`nvidia-smi`,
  `/sys/class/thermal`); Windows CPU runs report `null`.
- `python -I -c` code execution is isolation-by-flag, **not a security
  sandbox** — run only in trusted environments (your machine / Kaggle).
- Seed catalog filenames follow common quantizer naming conventions; the
  runner resolves actual files through the HF tree API at download time.