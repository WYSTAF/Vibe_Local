# Vibe Local — Architecture

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

This document describes the system design of Vibe Local: a zero-cloud local inference manager, hardware-aware MoE-vs-Dense recommender, 10-pillar GGUF quantization benchmark suite and Android/PC ecosystem node for **Vibe Router** and **Vibe Assembly**.

## 1. Workspace topology

```
                        ┌────────────────────────────────────────────┐
                        │                vibe-local-cli              │
                        │  main.rs (clap)   server.rs (Vibe-Link)    │
                        │  discovery.rs (UDP + mDNS)   commands.rs   │
                        └──────────────┬─────────────────────────────┘
                                       │ depends on
                 ┌─────────────────────┴──────────────────────┐
                 ▼                                            ▼
   ┌──────────────────────────┐              ┌──────────────────────────────┐
   │     vibe-local-engine    │              │        vibe-local-core       │
   │ supervisor (llama-server)│─────────────▶│ hardware  recommender        │
   │ microeval (deterministic)│  depends on  │ hf_crawler  gguf parser      │
   │ thermal governor         │              │ catalog math  db (SQLite)    │
   │ vibemark (15s benchmark) │              │ util (FNV-1a, time, RSS)     │
   └──────────────────────────┘              └──────────────────────────────┘
                 ▲                                            ▲
                 │              ┌─────────────────────────────┘
                 │              │
   ┌─────────────┴──────────────┴───┐
   │        vibe-local-jni          │   cdylib / staticlib / rlib
   │ JNI: Java_com_vibelocal_app_   │   Kotlin ↔ JSON string contract
   │ core_VibeLocalBridge_*         │   C-ABI: vibe_local_* + free_string
   └────────────────────────────────┘
                 ▲
                 │ System.loadLibrary("vibe_local_jni")
   ┌─────────────┴──────────────────┐
   │  android/ (Kotlin + Compose)   │   4 tabs · 4 safety guards
   └────────────────────────────────┘

   benchmarks/kaggle_10pillar_bench.py ──▶ SQLite (same schema) + POST /api/leaderboard/submit
   leaderboard-web/ (vanilla JS)       ◀── GET /api/leaderboard · /catalog.json
```

Every crate declares `license = "Apache-2.0"` and `edition = "2024"` via `[workspace.package]` inheritance.

## 2. Core crate (`vibe-local-core`)

### 2.1 HardwareProbe (`hardware.rs`)

Real OS reads, no hardcoded fallbacks:

| Signal | Windows | Linux / Android | macOS |
|---|---|---|---|
| RAM total/free | `GlobalMemoryStatusEx` (kernel32 FFI) | `/proc/meminfo` (`MemAvailable`, fallback `MemFree+Buffers+Cached`) | `sysctl hw.memsize` + `vm_stat` pages |
| CPU model | Registry `ProcessorNameString` | `/proc/cpuinfo` | `sysctl machdep.cpu.brand_string` |
| CPU flags | `is_x86_feature_detected!` (SSE4.2/AVX/AVX2/FMA/AVX512F/BW/VL) | same + aarch64 `neon/asimd/dotprod/i8mm/fp16` | same |
| GPU + VRAM | Registry display-class `DriverDesc` + `HardwareInformation.qwMemorySize` (QWORD — avoids the 32-bit `AdapterRAM` truncation); `LoadLibraryW` probes for `vulkan-1.dll` / `nvcuda.dll` | `/sys/class/drm/card*/device/{vendor,device,mem_info_vram_total}`, `lspci`, `nvidia-smi`, `dlopen(libvulkan/libcuda)`; Android Adreno/Mali sysfs paths | `system_profiler SPDisplaysDataType -json`, Metal always |
| Portable USB | `GetLogicalDrives` + `GetDriveTypeW == DRIVE_REMOVABLE` | `/media`, `/run/media`, `/mnt`, `/storage` cross-checked against `/proc/mounts` | `/Volumes` |

`VIBE_PORTABLE_ROOT` overrides the portable locator on every platform. Portable layout: `<usb>/portable_data/models/` and `<usb>/portable_data/vibe_local.db`. Non-portable default: `%LOCALAPPDATA%/vibe-local`, `$XDG_DATA_HOME/vibe-local`, or `~/Library/Application Support/vibe-local` (override: `VIBE_DATA_DIR`).

### 2.2 Recommender (`recommender.rs`)

Decision order:

1. **`gpu_dense`** — a Vulkan/CUDA GPU with ≥6GB VRAM → dense 7–8B at `Q4_K_M`/`Q5_K_M`, `-ngl 99`, weights+KV+compute must fit VRAM with 512MB headroom.
2. **`edge_compact`** — <2.5GB free RAM → dense 1.5–2B at `Q4_K_M`/`IQ4_XS` (LMKD-safe).
3. **`moe_sparse`** — ≥8GB total RAM, bandwidth/compute-constrained → sparse MoE (OLMoE 6.9B/1.3B-active, Granite 3.3B/0.8B-active, Qwen1.5-MoE 14.3B/2.7B-active); total weights + overhead + 1GB headroom must fit free RAM.

Sizes derive from quant bits-per-weight tables; every candidate carries `reasons[]`, `est_ram_mb`, `fit`, `ngl` and an estimated decode speedup vs dense-7B (clamped 1–3×, labeled as estimate). The curated registry holds public parameter counts; HF discovery enriches it with live files.

### 2.3 HF crawler (`hf_crawler.rs`)

- `GET /api/models?search=…&filter=gguf&sort=downloads` → repos ranked with trusted quantizers first (`bartowski`, `unsloth`, `ggml-org`, `MaziyarPanahi`).
- `GET /api/models/<repo>/tree/main?recursive=true` → `.gguf` files with LFS sizes, quant tags parsed from filenames, direct `resolve/main` download URLs.
- **SSRF guard**: https-only; host allowlist (`huggingface.co`, `cdn-lfs*.huggingface.co`, `cas-bridge.xethub.hf.co`); DNS resolution with rejection of loopback / link-local / RFC1918 / ULA / CGNAT / multicast / documentation ranges (IPv4 + IPv6, incl. IPv4-mapped); redirects followed manually and re-validated per hop (≤5). Downloads stream to `*.part` then atomic rename, 16GB ceiling.

### 2.4 GGUF parser (`gguf.rs`)

Zero-dependency (`std` only) little-endian reader: magic `GGUF`, versions 1–3, tensor count, full KV metadata (all 13 value types incl. nested arrays), `general.architecture`, `<arch>.context_length / block_count / embedding_length / attention.head_count{,_kv} / expert_count / expert_used_count`, `general.file_type` → quant label map (Q2_K…Q8_0, IQ series, BF16). Safety budgets: 64MB header, 1MB strings, 1M array elements.

RAM calculator at 2k/4k/8k/16k: `weights(file size) + KV(2·layers·ctx·kv_heads·head_dim·2 bytes f16) + compute(128MB + 24MB/1k ctx)`.

### 2.5 Catalog math (`catalog.rs`)

- 10 pillar IDs; `vibe_quality_score` = weighted mean of the 8 quality pillars (weights .16/.12/.14/.10/.12/.12/.12/.12, sum 1.0).
- `power_per_parameter = vibe_quality_score / active_params_b`.
- Pareto frontier: minimize active params, maximize quality (strict dominance).
- `CatalogEntry::validate()` enforces ranges, family, provenance enum and HF-only download URLs — applied on every API submission and DB upsert.

### 2.6 Unified SQLite (`db.rs`)

One schema for CLI, server, JNI and the Python runner:

```
hardware_scans(id, ts, os, cpu_model, ram_total_mb, ram_free_mb, preferred_backend, tier, report_json)
models(id, hf_repo, filename, quant, size_bytes, …, UNIQUE(hf_repo, filename))
gguf_inspections(id, ts, path, architecture, quant, info_json)
benchmarks(id, ts, model_id, hf_repo, filename, quant, hardware_profile, backend, pillar, score, metrics_json, provenance, source_url)
vibemark_runs(id, ts, model_id, backend, engine, prefill_tps, decode_tps, ttft_ms, peak_ram_mb, duration_s, provenance)
catalog_entries(id, model_id, quant, hardware_profile, provenance, entry_json, updated_ts, UNIQUE(model_id, quant, hardware_profile, provenance))
settings(key, value)
```

`busy_timeout = 5000`, `journal_mode = WAL` on disk, `synchronous = NORMAL`, single-owner access via `Arc<Mutex<Db>>`. Leaderboard assembly deduplicates by (model, quant, profile) keeping the best provenance: **measured > submitted > seed_estimate**.

## 3. Engine crate (`vibe-local-engine`)

### 3.1 Supervisor (`supervisor.rs`)

- One worker thread owns all state; commands arrive over a **bounded `sync_channel(16)`**; replies use bounded `sync_channel(1)` with timeouts.
- `Load`: validates the GGUF header, then spawns `llama-server -m … --host 127.0.0.1 --port 20130 -ngl N -c CTX -np 1` with backend env (`CUDA_VISIBLE_DEVICES` / `GGML_VK_DEVICE`), polls `/health` up to 150s, kills the child on failure. Binary resolution: explicit > `VIBE_LLAMA_SERVER` > `PATH` > `./llama.cpp/build/bin`.
- **No binary → builtin micro-eval mode** (real deterministic engine, never a stub, always labeled).
- **Idle auto-unload**: `recv_timeout(1s)` loop tracks `last_activity`; after `idle_secs` (default 300) the child is killed and state cleared.
- Chat proxies to llama-server `/v1/chat/completions` (temperature 0) or runs micro-eval; measures elapsed + token count for `X-Vibe-Local-Tok-Per-Sec`.

### 3.2 Micro-eval (`microeval.rs`)

Deterministic extractive reasoner: shunting-yard arithmetic evaluator (+ − × ÷ % ^, parens, unary minus), frequency-ranked keyword extraction, sentence-overlap extractive answering, structured echo fallback. Same input → same output, always. Used for pipeline validation and as a measurable VibeMark workload when llama.cpp is absent. `strip_think()` removes `<think>…</think>` (incl. unclosed trailing blocks).

### 3.3 Thermal governor (`thermal.rs`)

Real sensors: Linux/Android `/sys/class/thermal/thermal_zone*/temp` (CPU/SoC zones, battery excluded), Windows WMI `MSAcpi_ThermalZoneTemperature` via PowerShell (tenths of Kelvin), macOS `osx-cpu-temp` when present. Android injects readings through JNI `setThermalReading`. States: **≥42°C Throttled** (25ms inter-token delay, halved max_tokens), **≥45°C Paused** (requests rejected), resume <43°C (hysteresis). No sensor → `NoSensor`, never fabricated values.

### 3.4 VibeMark (`vibemark.rs`)

15-second budget (clamped 5–120): prefill probe (~512-token fixed prompt, `n_predict=1`, llama.cpp `timings.prompt_ms`), streaming TTFT probe (time to first SSE content chunk), decode probe sized to the remaining budget. Peak RAM sampled from the child process (`K32GetProcessMemoryInfo` / `/proc/<pid>/status` VmHWM / `ps`). Output: `{prefill_tps, decode_tps, ttft_ms, peak_ram_mb, duration_s, provenance:"measured"}`.

## 4. CLI + Vibe-Link server (`vibe-local-cli`)

### 4.1 HTTP surface (`server.rs`, tiny_http, 4 workers)

| Route | Behavior |
|---|---|
| `GET /api/hardware` | fresh probe + persisted scan |
| `GET /api/recommendations` | tier + ranked candidates |
| `GET /api/status` | engine/thermal/idle/LAN state |
| `GET /api/leaderboard?profile=` | entries + derived scores + Pareto mask + profiles |
| `POST /api/leaderboard/submit` | validated entry upsert (rejects `seed_estimate`), per-pillar audit rows |
| `GET /api/discover?query=` / `GET /api/models?query=` | live HF crawl / cached DB |
| `POST /api/load` / `POST /api/unload` / `POST /api/idle` / `POST /api/thermal` | engine control |
| `POST /api/vibemark` / `GET /api/vibemark/latest` | benchmark run + persistence |
| `GET /v1/models`, `POST /v1/chat/completions` | OpenAI-compatible; `strip_think` body flag or `X-Vibe-Strip-Think: 1`; router headers on every response |
| `GET /`, `/leaderboard`, static assets | web UI with strict CSP + traversal protection |
| `GET /catalog.json` | bundled seed catalog (or DB-synthesized) |

**Security middleware**: LAN mode binds `0.0.0.0` and requires `X-Vibe-Local-Token` / `Bearer` (constant-time compare) on all `/api/*` + `/v1/*`; LAN without a token rejects everything; mutating POST/DELETE require `Content-Type: application/json` or `X-Vibe-Admin: 1`. Loopback mode binds `127.0.0.1` only. CSP: `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`. Bodies capped at 8MB.

### 4.2 Discovery (`discovery.rs`)

- **UDP probe**: port 20131, `VIBE_DISCOVER_V1` → `{service, version, http_port, lan, host}`.
- **mDNS/DNS-SD**: hand-rolled DNS wire format on 224.0.0.251:5353 — answers PTR/SRV/TXT/A for `_vibe-local._tcp.local` (instance `Vibe Local (<host>)`, TXT `path=/`, `lan=0|1`, `version=…`). Query parser validates question names; responses sent unicast to the querier and to the multicast group.

### 4.3 CLI (`main.rs`)

`scan`, `discover`, `inspect-gguf`, `bench` (resolves HF ids → trusted quant download → VibeMark → optional `--submit`), `serve` (`--port/--lan/--token/--idle`, global `--portable/--db`), `leaderboard` (live server first, local DB fallback). `bench --submit` merges quality pillars from the matching seed entry **labeled per-pillar `source: seed_estimate`**, while throughput pillars are genuinely measured.

## 5. JNI bridge (`vibe-local-jni`)

`cdylib` + `staticlib` + `rlib`. All `Java_com_vibelocal_app_core_VibeLocalBridge_*` functions return JSON strings (`{"ok":true,"data":…}` / `{"ok":false,"error":…}`), wrap panics via `catch_unwind`, and share one global `BridgeState` (DB + engine + server handle). The Android app runs the **same server code** as the PC. C-ABI mirrors (`vibe_local_*`) with explicit `vibe_local_free_string` ownership.

## 6. Android app (`android/`)

Kotlin + Jetpack Compose, Material 3 Obsidian Dark (`#090A0F` / `#131620`). Tabs: Scan & Match, 10-Pillar Leaderboard, VibeMark, Chat & Vibe-Link LAN Worker. Guards: `LmkdMemoryGuard` (MemoryInfo polling + 512MB margin + trim-memory unload), `ThermalGovernor` (PowerManager listener → native `setThermalReading`), `VibeLinkForegroundService` (dataSync, notification, wake lock), `AutoUnloadIdleTimer` (300s default). LAN toggle generates a `SecureRandom` token and starts the token-gated server; HTTP fallback when the native lib is unavailable.

## 7. Benchmark suite (`benchmarks/`)

Stdlib-only Python (Kaggle-ready): HF resolver with the same SSRF policy, llama-server lifecycle, 10 real pillar scorers over embedded subsets (executed code tests, seeded needle haystacks, FNV-1a anchors, KL/perplexity with honest `retention_proxy` fallback), ctypes peak-RSS, WAL SQLite writes into the unified schema, `--submit` POSTs measured entries. `--self-test` runs 15 offline checks. The seed catalog (70 entries) is explicitly `seed_estimate` — illustrative, never presented as measurement.

## 8. Web leaderboard (`leaderboard-web/`)

Vanilla JS under strict CSP: zero `innerHTML`, all DOM via `createElement`/`textContent`. Canvas Pareto scatter (Power per Parameter), quant degradation curves Q8_0→Q2_K, hardware profile switcher, sortable 10-pillar matrix with provenance badges and HF download links, measured-run submission form. Live mode (`/api/leaderboard`) with bundled-catalog fallback (`/catalog.json`).

## 9. Data flow: a benchmark run

```
vibe-local bench Qwen2.5-Coder-7B
  → hf_crawler.discover (SSRF-guarded) → pick trusted Q4_K_M → download to models/
  → gguf.parse_file (header validation + RAM estimates) → gguf_inspections
  → engine.load (llama-server or builtin) → engine.vibemark(15s)
  → vibemark_runs + benchmarks(hardware_throughput, latency_thermal_efficiency)
  → catalog_entries(measured)  [quality pillars inherited from seeds, labeled]
  → optional POST /api/leaderboard/submit → remote Vibe-Link node
  → web dashboard re-renders Pareto frontier with measured > submitted > seed
```

## 10. Verification

`cargo fmt --all -- --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` · `cargo build --release` · `python benchmarks/kaggle_10pillar_bench.py --self-test`. Tests cover GGUF parsing (synthetic binaries), SSRF classification, tier decisions for the three reference devices, Pareto math, DB WAL/busy-timeout, LAN auth/CSRF enforcement, router headers, think-stripping, thermal hysteresis, mDNS wire format and the JNI/C-ABI surface.