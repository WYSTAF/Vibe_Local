/**
 * Pure data logic: pillar definitions, derived scores, Pareto frontier.
 * No DOM access — unit-testable in isolation.
 */

export const PILLARS = [
  'code_generation',
  'systems_polyglot',
  'math_reasoning',
  'hard_science_gpqa',
  'instruction_schema',
  'long_context_needle',
  'truthfulness_anti_sycophancy',
  'quant_retention_kl',
  'hardware_throughput',
  'latency_thermal_efficiency',
];

export const QUALITY_WEIGHTS = {
  code_generation: 0.16,
  systems_polyglot: 0.12,
  math_reasoning: 0.14,
  hard_science_gpqa: 0.10,
  instruction_schema: 0.12,
  long_context_needle: 0.12,
  truthfulness_anti_sycophancy: 0.12,
  quant_retention_kl: 0.12,
};

export const QUANT_ORDER = ['Q8_0', 'Q6_K', 'Q5_K_M', 'IQ4_NL', 'Q4_K_M', 'IQ4_XS', 'Q3_K_M', 'Q2_K'];

export const DEFAULT_PILLAR_LABELS = {
  code_generation: 'Code Generation',
  systems_polyglot: 'Systems / Polyglot',
  math_reasoning: 'Math Reasoning',
  hard_science_gpqa: 'Hard Science (GPQA)',
  instruction_schema: 'Instruction & Schema',
  long_context_needle: 'Long-Context Needle',
  truthfulness_anti_sycophancy: 'Truthfulness / Anti-Sycophancy',
  quant_retention_kl: 'Quant Retention (KL)',
  hardware_throughput: 'Hardware Throughput',
  latency_thermal_efficiency: 'Latency & Thermal Efficiency',
};

export const FALLBACK_PROFILES = {
  rx580_vulkan: { label: 'AMD RX 580 8GB Vulkan', backend: 'Vulkan', vram_mb: 8192, ram_mb: 32768 },
  nothing_phone_2a: { label: 'Nothing Phone 2a 12GB', backend: 'CPU (ARM)', vram_mb: 0, ram_mb: 12288 },
  i5_4690_ddr3: { label: 'i5-4690 16GB DDR3', backend: 'CPU (AVX2)', vram_mb: 0, ram_mb: 16384 },
  tesla_t4: { label: 'Tesla T4 CUDA', backend: 'CUDA', vram_mb: 16384, ram_mb: 65536 },
};

export const DEFAULT_PROFILE_ID = 'rx580_vulkan';

export function quantIndex(quant) {
  const i = QUANT_ORDER.indexOf(quant);
  return i === -1 ? QUANT_ORDER.length : i;
}

export function pillarScore(entry, pillarId) {
  const s = entry && entry.pillars && entry.pillars[pillarId] && entry.pillars[pillarId].score;
  return typeof s === 'number' && Number.isFinite(s) ? s : null;
}

/** Weighted mean of the 8 quality pillars (renormalized over available pillars). */
export function vibeQualityScore(entry) {
  let sum = 0;
  let wsum = 0;
  for (const [pid, w] of Object.entries(QUALITY_WEIGHTS)) {
    const s = pillarScore(entry, pid);
    if (s !== null) {
      sum += s * w;
      wsum += w;
    }
  }
  return wsum > 0 ? sum / wsum : null;
}

export function powerPerParameter(entry) {
  const vq = entry.vibe_quality_score;
  const active = entry.active_params_b;
  if (typeof vq !== 'number' || !Number.isFinite(vq)) return null;
  if (typeof active !== 'number' || !Number.isFinite(active) || active <= 0) return null;
  return vq / active;
}

/**
 * Fill derived fields when the server did not provide them.
 * Mutates and returns the entry (server-provided values win).
 */
export function deriveEntry(entry) {
  if (typeof entry.vibe_quality_score !== 'number' || !Number.isFinite(entry.vibe_quality_score)) {
    entry.vibe_quality_score = vibeQualityScore(entry);
  }
  if (typeof entry.power_per_parameter !== 'number' || !Number.isFinite(entry.power_per_parameter)) {
    entry.power_per_parameter = powerPerParameter(entry);
  }
  return entry;
}

export function deriveEntries(entries) {
  return entries.map(deriveEntry);
}

/**
 * Pareto frontier on (active_params_b ↓ better, vibe_quality_score ↑ better).
 * Sets `pareto_optimal` on every entry; returns the frontier sorted by active params.
 */
export function computePareto(entries) {
  const pts = entries.filter(e =>
    typeof e.active_params_b === 'number' && Number.isFinite(e.active_params_b) &&
    typeof e.vibe_quality_score === 'number' && Number.isFinite(e.vibe_quality_score));

  for (const e of entries) e.pareto_optimal = false;

  const frontier = [];
  for (const e of pts) {
    let dominated = false;
    for (const o of pts) {
      if (o === e) continue;
      if (o.active_params_b <= e.active_params_b &&
          o.vibe_quality_score >= e.vibe_quality_score &&
          (o.active_params_b < e.active_params_b || o.vibe_quality_score > e.vibe_quality_score)) {
        dominated = true;
        break;
      }
    }
    if (!dominated) {
      e.pareto_optimal = true;
      frontier.push(e);
    }
  }
  frontier.sort((a, b) => a.active_params_b - b.active_params_b || b.vibe_quality_score - a.vibe_quality_score);
  return frontier;
}

/** Filter catalog entries for one hardware profile (client-side fallback path). */
export function filterByProfile(entries, profileId) {
  return entries.filter(e => e.hardware_profile === profileId);
}

/** Group entries by model_id, each group sorted along the quant ladder. */
export function groupByModel(entries) {
  const map = new Map();
  for (const e of entries) {
    if (!map.has(e.model_id)) map.set(e.model_id, []);
    map.get(e.model_id).push(e);
  }
  for (const list of map.values()) {
    list.sort((a, b) => quantIndex(a.quant) - quantIndex(b.quant));
  }
  return map;
}

/**
 * Per-model quant series: [{quant, score, entries}] averaged across duplicate
 * entries for the same quant (e.g. repeated runs).
 */
export function quantSeries(entries) {
  const byQuant = new Map();
  for (const e of entries) {
    const s = e.vibe_quality_score;
    if (typeof s !== 'number' || !Number.isFinite(s)) continue;
    if (!byQuant.has(e.quant)) byQuant.set(e.quant, { sum: 0, n: 0, entries: [] });
    const b = byQuant.get(e.quant);
    b.sum += s;
    b.n += 1;
    b.entries.push(e);
  }
  return [...byQuant.entries()]
    .map(([quant, b]) => ({ quant, score: b.sum / b.n, entries: b.entries }))
    .sort((a, b) => quantIndex(a.quant) - quantIndex(b.quant));
}

export function entryKey(e) {
  return `${e.model_id}|${e.quant}|${e.hardware_profile}|${e.filename || ''}`;
}

/** Normalize a server/catalog payload into the internal shape. */
export function normalizePayload(raw, mode) {
  const profiles = raw.profiles || raw.hardware_profiles || FALLBACK_PROFILES;
  const pillars = Array.isArray(raw.pillars) && raw.pillars.length ? raw.pillars : PILLARS;
  const labels = { ...DEFAULT_PILLAR_LABELS, ...(raw.pillar_labels || {}) };
  const entries = Array.isArray(raw.entries) ? raw.entries.filter(e => e && typeof e === 'object') : [];
  return { mode, profiles, pillars, labels, entries };
}