/**
 * Vibe Local — Leaderboard application bootstrap.
 * Single page, four tab views, hardware-profile switcher, live/catalog data modes.
 * All DOM is built with createElement/textContent (strict CSP, no inline code).
 */
import { el, clear, debounce } from './js/dom.js';
import * as fmt from './js/format.js';
import {
  DEFAULT_PROFILE_ID, DEFAULT_PILLAR_LABELS, FALLBACK_PROFILES, QUALITY_WEIGHTS,
  deriveEntries, computePareto, filterByProfile, normalizePayload, entryKey,
} from './js/data.js';
import { fetchLeaderboard } from './js/api.js';
import { createScatterView } from './js/scatter.js';
import { createDegradationView } from './js/degradation.js';
import { createMatrixView } from './js/matrix.js';
import { createDetailDrawer } from './js/detail.js';
import { createSubmitView } from './js/submit.js';

const TABS = ['pareto', 'degradation', 'matrix', 'submit'];

const state = {
  mode: null,            // 'live' | 'catalog'
  profileId: DEFAULT_PROFILE_ID,
  payload: null,         // normalized {profiles, pillars, labels, entries(all)}
  entries: [],           // derived + filtered for current profile
  activeTab: 'pareto',
  loaded: false,
};

let fetchController = null;

// ----- static DOM refs -----
const $ = (id) => document.getElementById(id);
const profileSelect = $('profile-select');
const profileMeta = $('profile-meta');
const banner = $('connection-banner');
const tablist = $('tablist');
const tabInk = $('tab-ink');
const statsStrip = $('stats-strip');
const loadingState = $('loading-state');
const errorState = $('error-state');
const errorMessage = $('error-message');
const retryButton = $('retry-button');
const panels = Object.fromEntries(TABS.map(t => [t, $(`panel-${t}`)]));
const tabButtons = Object.fromEntries(TABS.map(t => [t, $(`tab-${t}`)]));

// ----- detail drawer -----
const drawer = createDetailDrawer($('detail-root'));

function openEntry(entry) {
  drawer.open(entry, {
    labels: state.payload ? state.payload.labels : DEFAULT_PILLAR_LABELS,
    profiles: state.payload ? state.payload.profiles : FALLBACK_PROFILES,
    pillars: state.payload ? state.payload.pillars : undefined,
  }, () => matrix.setSelected(null));
  matrix.setSelected(entryKey(entry));
}

// ----- views -----
const scatter = createScatterView(panels.pareto, { onOpenEntry: openEntry });
const degradation = createDegradationView(panels.degradation);
const matrix = createMatrixView(panels.matrix, { onOpenEntry: openEntry });
const submitView = createSubmitView(panels.submit, {
  onSuccess: () => {
    // Refresh live data so an accepted submission shows up immediately.
    if (state.mode === 'live') load(state.profileId, { silent: true });
  },
});
const views = { pareto: scatter, degradation, matrix, submit: submitView };

// ----- banner -----
function renderBanner(kind, message) {
  clear(banner);
  banner.className = `banner banner-${kind}`;
  banner.appendChild(el('span', { class: 'banner-dot', 'aria-hidden': 'true' }));
  if (kind === 'live') {
    banner.appendChild(el('strong', { text: 'Live server' }));
    banner.appendChild(el('span', { text: ' — connected to the Vibe Local API; entries filtered server-side per profile.' }));
    banner.appendChild(el('span', { class: 'banner-spacer' }));
    banner.appendChild(el('code', { text: `GET /api/leaderboard?profile=${state.profileId}` }));
  } else if (kind === 'catalog') {
    banner.appendChild(el('strong', { text: 'Bundled catalog' }));
    banner.appendChild(el('span', { text: ' — offline mode: reading catalog.json and filtering client-side. Seed values are illustrative estimates.' }));
    banner.appendChild(el('span', { class: 'banner-spacer' }));
    const retry = el('button', { type: 'button', class: 'btn btn-sm btn-ghost', text: 'Try live server' });
    retry.addEventListener('click', () => load(state.profileId));
    banner.appendChild(retry);
  } else if (kind === 'error') {
    banner.appendChild(el('strong', { text: 'Connection failed' }));
    banner.appendChild(el('span', { text: ` — ${message}` }));
    banner.appendChild(el('span', { class: 'banner-spacer' }));
    const retry = el('button', { type: 'button', class: 'btn btn-sm btn-primary', text: 'Retry' });
    retry.addEventListener('click', () => load(state.profileId));
    banner.appendChild(retry);
  } else {
    banner.appendChild(el('span', { text: 'Connecting to Vibe Local…' }));
  }
}

// ----- profile switcher -----
function syncProfileOptions() {
  const profiles = state.payload ? state.payload.profiles : FALLBACK_PROFILES;
  const ids = Object.keys(profiles);
  if (!ids.length) return;
  if (!ids.includes(state.profileId)) state.profileId = ids[0];
  clear(profileSelect);
  for (const id of ids) {
    profileSelect.appendChild(el('option', { value: id, text: profiles[id].label || id }));
  }
  profileSelect.value = state.profileId;
  const p = profiles[state.profileId] || {};
  const parts = [];
  if (p.backend) parts.push(p.backend);
  if (p.vram_mb) parts.push(`VRAM ${fmt.mb(p.vram_mb)}`);
  if (p.ram_mb) parts.push(`RAM ${fmt.mb(p.ram_mb)}`);
  profileMeta.textContent = parts.join(' · ');
}

profileSelect.addEventListener('change', () => {
  state.profileId = profileSelect.value;
  load(state.profileId);
});

// ----- stats strip -----
function renderStats() {
  clear(statsStrip);
  const entries = state.entries;
  if (!entries.length) return;
  const models = new Set(entries.map(e => e.model_id)).size;
  const frontier = entries.filter(e => e.pareto_optimal);
  const bestPower = entries.filter(e => typeof e.power_per_parameter === 'number')
    .sort((a, b) => b.power_per_parameter - a.power_per_parameter)[0];
  const bestVibe = entries.filter(e => typeof e.vibe_quality_score === 'number')
    .sort((a, b) => b.vibe_quality_score - a.vibe_quality_score)[0];
  const decodes = entries.map(e => e.throughput && e.throughput.decode_tps)
    .filter(v => typeof v === 'number').sort((a, b) => a - b);
  const medianDecode = decodes.length ? decodes[Math.floor(decodes.length / 2)] : null;

  const card = (label, value, unit, note, cls) => el('div', { class: `stat-card ${cls || ''}` },
    el('div', { class: 'stat-label', text: label }),
    el('div', { class: 'stat-value' }, value, unit ? el('span', { class: 'unit', text: unit }) : null),
    el('div', { class: 'stat-note', text: note, title: note }));

  statsStrip.appendChild(card('Runs in view', String(entries.length), '', `${models} distinct models`, 'stat-cyan'));
  statsStrip.appendChild(card('Pareto-optimal', String(frontier.length), '', 'non-dominated on params × quality', 'stat-violet'));
  statsStrip.appendChild(card('Best power/param', fmt.score(bestPower && bestPower.power_per_parameter), '/B',
    bestPower ? `${bestPower.model_id} @ ${bestPower.quant}` : '—', 'stat-violet'));
  statsStrip.appendChild(card('Top vibe score', fmt.score(bestVibe && bestVibe.vibe_quality_score), '',
    bestVibe ? `${bestVibe.model_id} @ ${bestVibe.quant}` : '—', 'stat-green'));
  statsStrip.appendChild(card('Median decode', fmt.tps(medianDecode), 'tok/s', 'across all runs in view', 'stat-amber'));
}

// ----- footer weights -----
function renderFooterWeights() {
  const box = $('footer-weights');
  clear(box);
  box.appendChild(el('span', { class: 'weight-chip', text: 'vibe_quality_score = Σ wᵢ·pillarᵢ' }));
  for (const [pid, w] of Object.entries(QUALITY_WEIGHTS)) {
    box.appendChild(el('span', { class: 'weight-chip' },
      `${(DEFAULT_PILLAR_LABELS[pid] || pid)} `, el('b', { text: w.toFixed(2) })));
  }
  box.appendChild(el('span', { class: 'weight-chip', text: 'power_per_parameter = vibe ÷ active_B' }));
}

// ----- tabs -----
function positionInk() {
  const btn = tabButtons[state.activeTab];
  if (!btn) return;
  tabInk.style.left = `${btn.offsetLeft}px`;
  tabInk.style.width = `${btn.offsetWidth}px`;
}

function activateTab(tab, { focus = false } = {}) {
  if (!TABS.includes(tab)) tab = 'pareto';
  state.activeTab = tab;
  for (const t of TABS) {
    const selected = t === tab;
    tabButtons[t].setAttribute('aria-selected', String(selected));
    tabButtons[t].tabIndex = selected ? 0 : -1;
    panels[t].hidden = !selected;
  }
  positionInk();
  if (focus) tabButtons[tab].focus();
  try {
    history.replaceState(null, '', `#${tab}`);
  } catch { /* file:// or sandboxed — ignore */ }
  // Canvas views need a layout pass once visible.
  requestAnimationFrame(() => {
    const view = views[tab];
    if (view && typeof view.refresh === 'function') view.refresh();
    positionInk();
  });
}

for (const t of TABS) {
  tabButtons[t].addEventListener('click', () => activateTab(t));
}

tablist.addEventListener('keydown', (ev) => {
  const idx = TABS.indexOf(state.activeTab);
  let next = null;
  if (ev.key === 'ArrowRight') next = TABS[(idx + 1) % TABS.length];
  else if (ev.key === 'ArrowLeft') next = TABS[(idx - 1 + TABS.length) % TABS.length];
  else if (ev.key === 'Home') next = TABS[0];
  else if (ev.key === 'End') next = TABS[TABS.length - 1];
  if (next) {
    ev.preventDefault();
    activateTab(next, { focus: true });
  }
});

window.addEventListener('resize', debounce(positionInk, 120));

// ----- data loading -----
function setBusyUI(busy) {
  loadingState.hidden = !busy;
  if (busy) {
    errorState.hidden = true;
    for (const t of TABS) panels[t].hidden = true;
    clear(statsStrip);
  }
}

function showError(message) {
  state.loaded = false;
  loadingState.hidden = true;
  errorState.hidden = false;
  errorMessage.textContent = message;
  for (const t of TABS) panels[t].hidden = true;
  clear(statsStrip);
  renderBanner('error', message);
}

async function load(profileId, { silent = false } = {}) {
  if (fetchController) fetchController.abort();
  fetchController = new AbortController();
  const signal = fetchController.signal;
  state.profileId = profileId;
  if (!silent) {
    setBusyUI(true);
    renderBanner('loading');
  }
  try {
    const { mode, data } = await fetchLeaderboard(profileId, signal);
    if (signal.aborted) return;
    const payload = normalizePayload(data, mode);
    state.mode = mode;
    state.payload = payload;

    const filtered = filterByProfile(payload.entries, profileId);
    state.entries = deriveEntries(filtered);
    computePareto(state.entries);
    state.loaded = true;

    syncProfileOptions();
    submitView.setContext(payload.labels, payload.profiles);
    submitView.setMode(mode);
    renderBanner(mode);
    renderStats();

    scatter.update(state.entries);
    degradation.update(state.entries);
    matrix.update(state.entries, payload.labels);

    setBusyUI(false);
    for (const t of TABS) panels[t].hidden = t !== state.activeTab;
    requestAnimationFrame(() => {
      const view = views[state.activeTab];
      if (view && typeof view.refresh === 'function') view.refresh();
      positionInk();
    });
  } catch (err) {
    if (signal.aborted || (err && err.name === 'AbortError')) return;
    showError(err && err.message ? err.message : String(err));
  }
}

retryButton.addEventListener('click', () => load(state.profileId));

// ----- boot -----
function boot() {
  renderFooterWeights();
  syncProfileOptions();
  const hash = location.hash.replace('#', '');
  if (TABS.includes(hash)) state.activeTab = hash;
  activateTab(state.activeTab);
  load(state.profileId);
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}