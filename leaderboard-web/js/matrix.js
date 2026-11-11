/**
 * View 3 — 10-Pillar Matrix: sortable, filterable table.
 * Heat-colored pillar cells (CSSOM background-color), provenance badges,
 * validated Hugging Face links, row click / Enter opens the detail drawer.
 */
import { el, clear, safeLink, provenanceBadge, familyBadge } from './dom.js';
import * as fmt from './format.js';
import { PILLARS, QUANT_ORDER, quantIndex, pillarScore, entryKey } from './data.js';

const PILLAR_SHORT = {
  code_generation: 'Code',
  systems_polyglot: 'Polyglot',
  math_reasoning: 'Math',
  hard_science_gpqa: 'GPQA',
  instruction_schema: 'Schema',
  long_context_needle: 'Needle',
  truthfulness_anti_sycophancy: 'Truth',
  quant_retention_kl: 'KL ret.',
  hardware_throughput: 'Thruput',
  latency_thermal_efficiency: 'Lat/Thm',
};

/** Red → amber → green heat scale, applied via element.style (CSP-safe CSSOM). */
export function heatColor(score) {
  if (typeof score !== 'number' || !Number.isFinite(score)) return null;
  const t = Math.min(1, Math.max(0, score));
  const stops = [
    [248, 113, 113], // red
    [245, 184, 76],  // amber
    [61, 220, 151],  // green
  ];
  const seg = t < 0.5 ? 0 : 1;
  const local = t < 0.5 ? t / 0.5 : (t - 0.5) / 0.5;
  const a = stops[seg];
  const b = stops[seg + 1];
  const mix = a.map((c, i) => Math.round(c + (b[i] - c) * local));
  const alpha = (0.13 + 0.2 * t).toFixed(3);
  return `rgba(${mix[0]}, ${mix[1]}, ${mix[2]}, ${alpha})`;
}

function buildColumns(labels) {
  const cols = [
    { key: 'model_id', label: 'Model', cls: 'col-model', type: 'text', get: e => e.model_id },
    { key: 'family', label: 'Family', type: 'text', get: e => e.family },
    { key: 'quant', label: 'Quant', type: 'quant', get: e => e.quant },
    { key: 'active_params_b', label: 'Active B', type: 'num', get: e => e.active_params_b, render: e => fmt.num(e.active_params_b, 2) },
    { key: 'size_gb', label: 'Size GB', type: 'num', get: e => fmt.gbValue(e.size_bytes), render: e => fmt.isNum(fmt.gbValue(e.size_bytes)) ? fmt.gbValue(e.size_bytes).toFixed(2) : '—' },
    { key: 'vibe_quality_score', label: 'Vibe', type: 'num', cls: 'strong vibe', get: e => e.vibe_quality_score, render: e => fmt.score(e.vibe_quality_score) },
    { key: 'power_per_parameter', label: 'Power/B', type: 'num', get: e => e.power_per_parameter, render: e => fmt.score(e.power_per_parameter) },
  ];
  for (const pid of PILLARS) {
    cols.push({
      key: `pillar:${pid}`,
      label: PILLAR_SHORT[pid] || pid,
      title: labels[pid] || pid,
      type: 'num',
      cls: 'col-pillar',
      pillar: pid,
      get: e => pillarScore(e, pid),
      render: e => fmt.score(pillarScore(e, pid), 2),
    });
  }
  cols.push(
    { key: 'decode_tps', label: 'Decode t/s', type: 'num', get: e => e.throughput && e.throughput.decode_tps, render: e => fmt.tps(e.throughput && e.throughput.decode_tps) },
    { key: 'ttft_ms', label: 'TTFT', type: 'num', get: e => e.throughput && e.throughput.ttft_ms, render: e => fmt.ms(e.throughput && e.throughput.ttft_ms) },
    { key: 'provenance', label: 'Source', type: 'text', get: e => e.provenance },
    { key: 'hf', label: 'HF', type: 'none', sortable: false },
  );
  return cols;
}

export function createMatrixView(container, { onOpenEntry }) {
  let entries = [];
  let labels = {};
  let columns = buildColumns(labels);
  const sort = { key: 'vibe_quality_score', dir: 'desc' };
  const filters = { search: '', family: 'all', quant: 'all', paretoOnly: false };
  let selectedKey = null;

  // ----- toolbar -----
  const searchInput = el('input', {
    type: 'search', id: 'matrix-search', placeholder: 'Search model, repo, filename…',
    'aria-label': 'Search entries by model, repository, filename, quant or architecture',
  });
  const familySelect = el('select', { id: 'matrix-family', 'aria-label': 'Filter by family' },
    el('option', { value: 'all', text: 'All families' }),
    el('option', { value: 'dense', text: 'Dense only' }),
    el('option', { value: 'moe', text: 'MoE only' }));
  const quantSelect = el('select', { id: 'matrix-quant', 'aria-label': 'Filter by quantization' },
    el('option', { value: 'all', text: 'All quants' }),
    ...QUANT_ORDER.map(q => el('option', { value: q, text: q })));
  const paretoCb = el('input', { type: 'checkbox', id: 'matrix-pareto' });
  const countLabel = el('span', { class: 'matrix-count', role: 'status', 'aria-live': 'polite' });

  const toolbar = el('div', { class: 'matrix-toolbar', role: 'group', 'aria-label': 'Matrix filters' },
    searchInput, familySelect, quantSelect,
    el('label', { class: 'toggle', for: 'matrix-pareto' }, paretoCb, 'Pareto only'),
    countLabel);

  searchInput.addEventListener('input', () => { filters.search = searchInput.value.trim().toLowerCase(); renderBody(); });
  familySelect.addEventListener('change', () => { filters.family = familySelect.value; renderBody(); });
  quantSelect.addEventListener('change', () => { filters.quant = quantSelect.value; renderBody(); });
  paretoCb.addEventListener('change', () => { filters.paretoOnly = paretoCb.checked; renderBody(); });

  // ----- table -----
  const thead = el('thead');
  const tbody = el('tbody');
  const table = el('table', { class: 'matrix', 'aria-label': '10-pillar benchmark matrix. Column headers sort; rows open details.' }, thead, tbody);
  const scroll = el('div', { class: 'table-scroll', tabindex: '0', role: 'region', 'aria-label': 'Scrollable benchmark matrix' }, table);

  container.appendChild(el('div', { class: 'panel-head' },
    el('div', {},
      el('h2', { class: 'panel-title', text: '10-Pillar Matrix — ' }, el('span', { class: 'accent', text: 'every run, every pillar' })),
      el('p', { class: 'panel-desc', text: 'Click a column header to sort (click again to reverse). Click a row — or focus it and press Enter — for per-pillar metrics, sample counts and throughput. Pillar cells are heat-colored from red (weak) to green (strong).' })),
  ));
  container.appendChild(toolbar);
  container.appendChild(scroll);

  function buildHead() {
    clear(thead);
    const tr = el('tr');
    for (const col of columns) {
      const th = el('th', {
        scope: 'col',
        class: col.cls || '',
        'aria-sort': 'none',
        ...(col.title ? { title: col.title } : {}),
      });
      if (col.sortable === false) {
        th.appendChild(el('span', { class: 'sort-btn', text: col.label, style: { cursor: 'default' } }));
      } else {
        const arrow = el('span', { class: 'arrow', 'aria-hidden': 'true', text: '▲' });
        const btn = el('button', {
          type: 'button',
          class: 'sort-btn',
          'aria-label': `Sort by ${col.title || col.label}`,
        }, col.label, arrow);
        btn.addEventListener('click', () => {
          if (sort.key === col.key) {
            sort.dir = sort.dir === 'asc' ? 'desc' : 'asc';
          } else {
            sort.key = col.key;
            sort.dir = col.type === 'text' ? 'asc' : 'desc';
          }
          renderBody();
        });
        th.appendChild(btn);
      }
      tr.appendChild(th);
    }
    thead.appendChild(tr);
  }

  function syncHead() {
    const ths = thead.querySelectorAll('th');
    columns.forEach((col, i) => {
      const th = ths[i];
      if (!th) return;
      const btn = th.querySelector('.sort-btn');
      const arrow = th.querySelector('.arrow');
      const active = sort.key === col.key && col.sortable !== false;
      th.setAttribute('aria-sort', active ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none');
      if (btn) btn.classList.toggle('sorted', active);
      if (arrow) arrow.textContent = active && sort.dir === 'desc' ? '▼' : '▲';
    });
  }

  function filtered() {
    return entries.filter(e => {
      if (filters.family !== 'all' && e.family !== filters.family) return false;
      if (filters.quant !== 'all' && e.quant !== filters.quant) return false;
      if (filters.paretoOnly && !e.pareto_optimal) return false;
      if (filters.search) {
        const hay = `${e.model_id} ${e.hf_repo || ''} ${e.filename || ''} ${e.quant} ${e.architecture || ''}`.toLowerCase();
        if (!hay.includes(filters.search)) return false;
      }
      return true;
    });
  }

  function compare(a, b) {
    const col = columns.find(c => c.key === sort.key) || columns[0];
    let va = col.get ? col.get(a) : a[sort.key];
    let vb = col.get ? col.get(b) : b[sort.key];
    if (col.type === 'quant') { va = quantIndex(va); vb = quantIndex(vb); }
    const aNull = va === null || va === undefined || va === '';
    const bNull = vb === null || vb === undefined || vb === '';
    if (aNull && bNull) return 0;
    if (aNull) return 1;   // nulls always last
    if (bNull) return -1;
    let cmp;
    if (col.type === 'text') cmp = String(va).localeCompare(String(vb));
    else cmp = va < vb ? -1 : va > vb ? 1 : 0;
    if (cmp === 0) cmp = String(a.model_id).localeCompare(String(b.model_id)) || quantIndex(a.quant) - quantIndex(b.quant);
    return sort.dir === 'asc' ? cmp : -cmp;
  }

  function renderBody() {
    syncHead();
    const rows = filtered().sort(compare);
    clear(tbody);
    countLabel.textContent = `${rows.length} of ${entries.length} runs`;

    if (!rows.length) {
      const tr = el('tr');
      const td = el('td', { colspan: String(columns.length), class: 'matrix-empty', text: entries.length ? 'No runs match the current filters.' : 'No entries for this hardware profile yet.' });
      tr.appendChild(td);
      tbody.appendChild(tr);
      return;
    }

    const frag = document.createDocumentFragment();
    for (const e of rows) {
      const key = entryKey(e);
      const tr = el('tr', {
        tabindex: '0',
        'aria-label': `${e.model_id} ${e.quant}, vibe score ${fmt.score(e.vibe_quality_score)}. Press Enter for details.`,
        ...(key === selectedKey ? { class: 'selected' } : {}),
      });
      tr.addEventListener('click', () => onOpenEntry(e));
      tr.addEventListener('keydown', (ev) => {
        if (ev.key === 'Enter' || ev.key === ' ') {
          ev.preventDefault();
          onOpenEntry(e);
        }
      });

      for (const col of columns) {
        let td;
        if (col.key === 'model_id') {
          td = el('td', { class: 'col-model' },
            el('div', { class: 'model-cell' },
              el('span', { class: 'model-name', text: e.model_id, title: e.model_id }),
              el('span', { class: 'model-file', text: e.filename || '', title: e.filename || '' })));
        } else if (col.key === 'family') {
          td = el('td', {}, familyBadge(e.family));
        } else if (col.key === 'provenance') {
          td = el('td', {}, provenanceBadge(e.provenance));
        } else if (col.key === 'hf') {
          td = el('td', {});
          const link = safeLink(e.download_url, 'HF ↗', 'hf-link');
          link.addEventListener('click', (ev) => ev.stopPropagation());
          td.appendChild(link);
        } else if (col.pillar) {
          const s = pillarScore(e, col.pillar);
          td = el('td', { class: 'num pillar-cell', title: `${col.title}: ${fmt.score(s)}` });
          const heat = heatColor(s);
          if (heat) td.style.backgroundColor = heat; // CSSOM — allowed under style-src 'self'
          else td.classList.add('pillar-missing');
          td.textContent = col.render(e);
        } else {
          td = el('td', { class: `num ${col.cls || ''}`, text: col.render ? col.render(e) : String(col.get(e) ?? '—') });
        }
        tr.appendChild(td);
      }
      frag.appendChild(tr);
    }
    tbody.appendChild(frag);
  }

  function update(nextEntries, nextLabels) {
    entries = nextEntries;
    if (nextLabels) {
      labels = nextLabels;
      columns = buildColumns(labels);
      buildHead();
    }
    renderBody();
  }

  function setSelected(key) {
    selectedKey = key;
    for (const tr of tbody.querySelectorAll('tr')) {
      tr.classList.remove('selected');
    }
    if (!key) return;
    const rows = filtered().sort(compare);
    const idx = rows.findIndex(e => entryKey(e) === key);
    if (idx >= 0 && tbody.children[idx]) tbody.children[idx].classList.add('selected');
  }

  buildHead();

  return { update, refresh: renderBody, setSelected, destroy() {} };
}