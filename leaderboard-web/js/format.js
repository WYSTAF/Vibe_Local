/** Number formatting helpers — every displayed number goes through these. */

const DASH = '—';

export function isNum(v) {
  return typeof v === 'number' && Number.isFinite(v);
}

export function score(v, digits = 3) {
  return isNum(v) ? v.toFixed(digits) : DASH;
}

export function num(v, digits = 1) {
  return isNum(v) ? v.toFixed(digits) : DASH;
}

export function int(v) {
  return isNum(v) ? Math.round(v).toLocaleString('en-US') : DASH;
}

/** Bytes → GB (binary GiB, labelled GB per GGUF convention). */
export function gb(bytes) {
  return isNum(bytes) ? `${(bytes / 1073741824).toFixed(2)} GB` : DASH;
}

export function gbValue(bytes) {
  return isNum(bytes) ? bytes / 1073741824 : null;
}

export function params(b) {
  return isNum(b) ? `${b.toFixed(2)} B` : DASH;
}

export function tps(v) {
  return isNum(v) ? `${v.toFixed(1)}` : DASH;
}

export function ms(v) {
  if (!isNum(v)) return DASH;
  return v < 1000 ? `${Math.round(v)} ms` : `${(v / 1000).toFixed(2)} s`;
}

export function mb(v) {
  if (!isNum(v)) return DASH;
  return v >= 1024 ? `${(v / 1024).toFixed(1)} GB` : `${Math.round(v)} MB`;
}

export function pct(v, digits = 1) {
  return isNum(v) ? `${(v * 100).toFixed(digits)}%` : DASH;
}

export function signed(v, digits = 3) {
  if (!isNum(v)) return DASH;
  const s = v.toFixed(digits);
  return v > 0 ? `+${s}` : s;
}

export function celsius(v) {
  return isNum(v) ? `${v > 0 ? '+' : ''}${v.toFixed(1)} °C` : DASH;
}

/** Short display name for a model id (strip vendor suffix noise). */
export function shortModel(modelId) {
  return String(modelId || 'unknown');
}

/** Render an arbitrary detail value as plain text (never HTML). */
export function detailValue(v) {
  if (v === null || v === undefined) return DASH;
  if (typeof v === 'number') return Number.isInteger(v) ? String(v) : String(Math.round(v * 1000) / 1000);
  if (typeof v === 'boolean') return v ? 'yes' : 'no';
  if (typeof v === 'string') return v;
  if (Array.isArray(v)) return v.map(detailValue).join(', ');
  if (typeof v === 'object') {
    return Object.entries(v).map(([k, val]) => `${k}: ${detailValue(val)}`).join(' · ');
  }
  return String(v);
}