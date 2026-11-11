/**
 * View 1 — Power per Parameter: Pareto scatter (canvas, DPR-aware).
 * x = active_params_b (log or linear), y = vibe_quality_score.
 * Color by family (MoE violet / Dense cyan), radius ~ decode_tps,
 * Pareto frontier drawn as a step line. Hover + keyboard tooltips.
 */
import { el, clear } from './dom.js';
import * as fmt from './format.js';
import { computePareto } from './data.js';
import {
  createCanvasHost, fitCanvas, observeResize, exportCanvasPng,
  attachKeyboardNav, hexToRgba, FAMILY_COLORS, AXIS, drawAxisLabel,
} from './chart-utils.js';

const MARGIN = { top: 26, right: 30, bottom: 54, left: 62 };

export function createScatterView(container, { onOpenEntry }) {
  const opts = { logX: true, frontier: true, labels: false, sizeBy: 'decode' };
  let entries = [];
  let frontier = [];
  let points = [];
  let hoverIndex = -1;
  let rafPending = false;
  let scales = null;

  // ----- toolbar -----
  const logToggle = el('input', { type: 'checkbox', id: 'scatter-log', checked: true });
  const frontierToggle = el('input', { type: 'checkbox', id: 'scatter-frontier', checked: true });
  const labelsToggle = el('input', { type: 'checkbox', id: 'scatter-labels' });
  const sizeSelect = el('select', { id: 'scatter-size', 'aria-label': 'Point size encodes' },
    el('option', { value: 'decode', text: 'Size: decode tok/s' }),
    el('option', { value: 'prefill', text: 'Size: prefill tok/s' }),
    el('option', { value: 'uniform', text: 'Size: uniform' }));
  const exportBtn = el('button', { type: 'button', class: 'btn btn-sm btn-ghost', text: '⤓ Export PNG' });

  const toolbar = el('div', { class: 'chart-toolbar', role: 'group', 'aria-label': 'Scatter plot controls' },
    el('span', { class: 'toolbar-title', text: 'Controls' }),
    el('label', { class: 'toggle', for: 'scatter-log' }, logToggle, 'Log x-axis'),
    el('label', { class: 'toggle', for: 'scatter-frontier' }, frontierToggle, 'Pareto frontier'),
    el('label', { class: 'toggle', for: 'scatter-labels' }, labelsToggle, 'Point labels'),
    sizeSelect,
    el('span', { class: 'toolbar-spacer' }),
    exportBtn);

  logToggle.addEventListener('change', () => { opts.logX = logToggle.checked; render(); });
  frontierToggle.addEventListener('change', () => { opts.frontier = frontierToggle.checked; render(); });
  labelsToggle.addEventListener('change', () => { opts.labels = labelsToggle.checked; render(); });
  sizeSelect.addEventListener('change', () => { opts.sizeBy = sizeSelect.value; render(); });
  exportBtn.addEventListener('click', () => exportCanvasPng(host.canvas, 'vibe-local-pareto.png'));

  // ----- canvas host -----
  const host = createCanvasHost('Pareto scatter plot: vibe quality score versus active parameters. Use arrow keys to explore points, Enter to open details.');
  const emptyMsg = el('div', { class: 'chart-empty', hidden: true },
    el('p', { text: 'No entries for this hardware profile. Try another profile or submit a measured run.' }));
  host.wrap.appendChild(emptyMsg);

  // ----- legend + annotation -----
  const moeCount = el('span', { class: 'legend-note' });
  const denseCount = el('span', { class: 'legend-note' });
  const sizeNote = el('span', { class: 'legend-note' });
  const legend = el('div', { class: 'chart-legend', 'aria-label': 'Chart legend' },
    el('span', { class: 'legend-item' }, el('span', { class: 'legend-swatch moe' }), 'MoE', moeCount),
    el('span', { class: 'legend-item' }, el('span', { class: 'legend-swatch dense' }), 'Dense', denseCount),
    el('span', { class: 'legend-item' }, el('span', { class: 'legend-line' }), 'Pareto frontier'),
    sizeNote);
  const annotation = el('div', { class: 'chart-annotation' });

  container.appendChild(el('div', { class: 'panel-head' },
    el('div', {},
      el('h2', { class: 'panel-title', text: 'Power per Parameter — ' }, el('span', { class: 'accent', text: 'Pareto frontier' })),
      el('p', { class: 'panel-desc', text: 'Quality per active billion parameters. Upper-left wins: fewer active parameters, higher vibe score. Point size encodes throughput; click a point (or press Enter while exploring) for the full pillar breakdown.' })),
  ));
  container.appendChild(toolbar);
  container.appendChild(host.wrap);
  container.appendChild(legend);
  container.appendChild(annotation);

  // ----- scales -----
  function computeScales(w, h) {
    const plotW = w - MARGIN.left - MARGIN.right;
    const plotH = h - MARGIN.top - MARGIN.bottom;
    const actives = entries.map(e => e.active_params_b).filter(v => typeof v === 'number' && v > 0);
    const scores = entries.map(e => e.vibe_quality_score).filter(v => typeof v === 'number');
    if (!actives.length || !scores.length) return null;

    const aMin = Math.min(...actives);
    const aMax = Math.max(...actives);
    const sMin = Math.min(...scores);
    const sMax = Math.max(...scores);

    let xMin, xMax;
    if (opts.logX) {
      xMin = Math.max(0.05, aMin * 0.72);
      xMax = aMax * 1.4;
    } else {
      xMin = 0;
      xMax = aMax * 1.08;
    }
    const yMin = Math.max(0, sMin - 0.06);
    const yMax = Math.min(1.0, sMax + 0.06);

    const xToPx = (v) => {
      const t = opts.logX
        ? (Math.log10(v) - Math.log10(xMin)) / (Math.log10(xMax) - Math.log10(xMin))
        : (v - xMin) / (xMax - xMin);
      return MARGIN.left + t * plotW;
    };
    const yToPx = (v) => MARGIN.top + (1 - (v - yMin) / (yMax - yMin || 1)) * plotH;
    return { plotW, plotH, xMin, xMax, yMin, yMax, xToPx, yToPx };
  }

  function xTicks(sc) {
    if (opts.logX) {
      const candidates = [0.1, 0.2, 0.5, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96];
      return candidates.filter(v => v >= sc.xMin && v <= sc.xMax);
    }
    const step = niceStep((sc.xMax - sc.xMin) / 6);
    const ticks = [];
    for (let v = 0; v <= sc.xMax + 1e-9; v += step) ticks.push(Math.round(v * 100) / 100);
    return ticks;
  }

  function yTicks(sc) {
    const step = niceStep((sc.yMax - sc.yMin) / 5);
    const ticks = [];
    for (let v = Math.ceil(sc.yMin / step) * step; v <= sc.yMax + 1e-9; v += step) {
      ticks.push(Math.round(v * 1000) / 1000);
    }
    return ticks;
  }

  function niceStep(raw) {
    const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
    const norm = raw / mag;
    if (norm <= 1) return mag;
    if (norm <= 2) return 2 * mag;
    if (norm <= 5) return 5 * mag;
    return 10 * mag;
  }

  function radiusFor(e, maxTps) {
    if (opts.sizeBy === 'uniform') return 5.5;
    const t = opts.sizeBy === 'prefill'
      ? (e.throughput && e.throughput.prefill_tps)
      : (e.throughput && e.throughput.decode_tps);
    if (typeof t !== 'number' || !Number.isFinite(t) || maxTps <= 0) return 4;
    return 3.2 + 8.5 * Math.sqrt(Math.max(0, t) / maxTps);
  }

  // ----- render -----
  function render() {
    if (!host.wrap.clientWidth) return; // hidden tab — skip
    const { ctx, w, h } = fitCanvas(host.canvas);
    points = [];
    scales = null;
    emptyMsg.hidden = entries.length > 0;
    if (!entries.length) {
      host.tooltip.hide();
      return;
    }
    const sc = computeScales(w, h);
    if (!sc) return;
    scales = sc;

    // grid + axes
    ctx.save();
    ctx.font = AXIS.font;
    ctx.lineWidth = 1;
    for (const t of xTicks(sc)) {
      const x = Math.round(sc.xToPx(t)) + 0.5;
      ctx.strokeStyle = AXIS.grid;
      ctx.beginPath();
      ctx.moveTo(x, MARGIN.top);
      ctx.lineTo(x, MARGIN.top + sc.plotH);
      ctx.stroke();
      ctx.fillStyle = AXIS.text;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillText(String(t), x, MARGIN.top + sc.plotH + 8);
    }
    for (const t of yTicks(sc)) {
      const y = Math.round(sc.yToPx(t)) + 0.5;
      ctx.strokeStyle = AXIS.grid;
      ctx.beginPath();
      ctx.moveTo(MARGIN.left, y);
      ctx.lineTo(MARGIN.left + sc.plotW, y);
      ctx.stroke();
      ctx.fillStyle = AXIS.text;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(t.toFixed(2), MARGIN.left - 9, y);
    }
    // plot border
    ctx.strokeStyle = AXIS.gridStrong;
    ctx.strokeRect(MARGIN.left + 0.5, MARGIN.top + 0.5, sc.plotW, sc.plotH);
    ctx.restore();

    drawAxisLabel(ctx, 'Active parameters (B' + (opts.logX ? ', log scale' : '') + ')', MARGIN.left + sc.plotW / 2, h - 14);
    ctx.save();
    ctx.translate(16, MARGIN.top + sc.plotH / 2);
    ctx.rotate(-Math.PI / 2);
    drawAxisLabel(ctx, 'Vibe quality score', 0, 0);
    ctx.restore();

    // Pareto frontier step line
    if (opts.frontier && frontier.length) {
      ctx.save();
      ctx.strokeStyle = 'rgba(230, 233, 242, 0.6)';
      ctx.lineWidth = 1.6;
      ctx.setLineDash([6, 5]);
      ctx.beginPath();
      ctx.moveTo(MARGIN.left, sc.yToPx(frontier[0].vibe_quality_score));
      for (let i = 0; i < frontier.length; i++) {
        const x = sc.xToPx(frontier[i].active_params_b);
        const y = sc.yToPx(frontier[i].vibe_quality_score);
        ctx.lineTo(x, i === 0 ? y : sc.yToPx(frontier[i - 1].vibe_quality_score));
        ctx.lineTo(x, y);
      }
      ctx.stroke();
      ctx.restore();
    }

    // points (large first so small ones stay clickable on top)
    const tpsKey = opts.sizeBy === 'prefill' ? 'prefill_tps' : 'decode_tps';
    const maxTps = Math.max(1, ...entries.map(e => (e.throughput && e.throughput[tpsKey]) || 0));
    const ordered = entries
      .map((e, i) => ({ e, i, r: radiusFor(e, maxTps) }))
      .sort((a, b) => b.r - a.r);

    for (const { e, r } of ordered) {
      if (typeof e.active_params_b !== 'number' || typeof e.vibe_quality_score !== 'number') continue;
      const x = sc.xToPx(e.active_params_b);
      const y = sc.yToPx(e.vibe_quality_score);
      const color = FAMILY_COLORS[e.family] || FAMILY_COLORS.dense;
      points.push({ x, y, r, entry: e });
    }
    // draw in reversed order (points[] is hit-test order: small on top)
    for (let i = points.length - 1; i >= 0; i--) {
      const p = points[i];
      const color = FAMILY_COLORS[p.entry.family] || FAMILY_COLORS.dense;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      ctx.fillStyle = hexToRgba(color, 0.78);
      ctx.fill();
      ctx.lineWidth = 1.2;
      ctx.strokeStyle = hexToRgba(color, 1);
      ctx.stroke();
      if (p.entry.pareto_optimal) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r + 3.2, 0, Math.PI * 2);
        ctx.lineWidth = 1.3;
        ctx.strokeStyle = 'rgba(230, 233, 242, 0.75)';
        ctx.stroke();
      }
    }

    // hover / keyboard highlight
    if (hoverIndex >= 0 && hoverIndex < points.length) {
      const p = points[hoverIndex];
      ctx.save();
      ctx.strokeStyle = 'rgba(230, 233, 242, 0.35)';
      ctx.setLineDash([3, 4]);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(MARGIN.left, p.y);
      ctx.lineTo(p.x, p.y);
      ctx.lineTo(p.x, MARGIN.top + sc.plotH);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r + 5.5, 0, Math.PI * 2);
      ctx.strokeStyle = '#E6E9F2';
      ctx.lineWidth = 1.8;
      ctx.stroke();
      ctx.restore();
    }

    // optional labels
    if (opts.labels) {
      ctx.save();
      ctx.font = AXIS.fontSmall;
      ctx.textBaseline = 'middle';
      for (const p of points) {
        const color = FAMILY_COLORS[p.entry.family] || FAMILY_COLORS.dense;
        ctx.fillStyle = hexToRgba(color, 0.95);
        ctx.textAlign = 'left';
        const label = `${p.entry.model_id} ${p.entry.quant}`;
        const short = label.length > 26 ? label.slice(0, 25) + '…' : label;
        ctx.fillText(short, p.x + p.r + 5, p.y);
      }
      ctx.restore();
    }

    // legend counts + size note
    const moe = entries.filter(e => e.family === 'moe').length;
    moeCount.textContent = `(${moe})`;
    denseCount.textContent = `(${entries.length - moe})`;
    if (opts.sizeBy === 'uniform') {
      sizeNote.textContent = 'Point size: uniform';
    } else {
      const vals = entries.map(e => e.throughput && e.throughput[tpsKey]).filter(v => typeof v === 'number');
      sizeNote.textContent = vals.length
        ? `Point size ∝ ${opts.sizeBy} tok/s (${fmt.num(Math.min(...vals))}–${fmt.num(Math.max(...vals))})`
        : 'Point size: no throughput data';
    }
    updateAnnotation();
  }

  function updateAnnotation() {
    clear(annotation);
    if (!entries.length) return;
    const best = entries
      .filter(e => typeof e.power_per_parameter === 'number')
      .sort((a, b) => b.power_per_parameter - a.power_per_parameter)[0];
    const moe = entries.filter(e => e.family === 'moe').length;
    annotation.appendChild(el('strong', { text: 'MoE vs Dense: ' }));
    annotation.appendChild(document.createTextNode(
      `MoE models (violet) activate only a fraction of their weights per token, so they sit further left at comparable quality — but their full file still has to fit in memory. Dense models (cyan) pay for every parameter on every token. `));
    if (best) {
      annotation.appendChild(el('strong', { text: 'Best power/parameter here: ' }));
      annotation.appendChild(document.createTextNode(
        `${best.model_id} @ ${best.quant} — ${fmt.score(best.power_per_parameter)} score per active B (${fmt.score(best.vibe_quality_score)} vibe, ${fmt.params(best.active_params_b)} active). `));
    }
    annotation.appendChild(document.createTextNode(
      `${frontier.length} of ${entries.length} runs are Pareto-optimal${moe ? `; ${moe} MoE runs in view` : ''}.`));
  }

  function scheduleRender() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      render();
    });
  }

  // ----- interaction -----
  function showTooltipFor(index, announce) {
    const p = points[index];
    if (!p) return;
    const e = p.entry;
    host.tooltip.show(p.x, p.y, e.model_id, `${e.quant} · ${e.family === 'moe' ? 'MoE' : 'Dense'} · ${e.architecture || '?'}`, [
      { k: 'Vibe score', v: fmt.score(e.vibe_quality_score), cls: 'cyan' },
      { k: 'Power / param', v: fmt.score(e.power_per_parameter), cls: 'violet' },
      { k: 'Decode', v: `${fmt.tps(e.throughput && e.throughput.decode_tps)} tok/s` },
      { k: 'Prefill', v: `${fmt.tps(e.throughput && e.throughput.prefill_tps)} tok/s` },
      { k: 'Active params', v: fmt.params(e.active_params_b) },
      { k: 'File size', v: fmt.gb(e.size_bytes) },
      { k: 'Pareto', v: e.pareto_optimal ? 'optimal ✓' : 'dominated', cls: e.pareto_optimal ? 'green' : '' },
    ]);
    if (announce) {
      host.live.textContent = `${e.model_id}, ${e.quant}. Vibe score ${fmt.score(e.vibe_quality_score)}, power per parameter ${fmt.score(e.power_per_parameter)}, decode ${fmt.tps(e.throughput && e.throughput.decode_tps)} tokens per second.${e.pareto_optimal ? ' Pareto optimal.' : ''}`;
    }
  }

  host.canvas.addEventListener('mousemove', (ev) => {
    const rect = host.canvas.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;
    let best = -1;
    let bestD = Infinity;
    for (let i = 0; i < points.length; i++) {
      const p = points[i];
      const d = Math.hypot(p.x - x, p.y - y);
      if (d <= Math.max(12, p.r + 5) && d < bestD) { bestD = d; best = i; }
    }
    if (best !== hoverIndex) {
      hoverIndex = best;
      scheduleRender();
    }
    if (best >= 0) {
      showTooltipFor(best, false);
      host.canvas.style.cursor = 'pointer';
    } else {
      host.tooltip.hide();
      host.canvas.style.cursor = 'crosshair';
    }
  });

  host.canvas.addEventListener('mouseleave', () => {
    if (hoverIndex !== -1) {
      hoverIndex = -1;
      scheduleRender();
    }
    host.tooltip.hide();
  });

  host.canvas.addEventListener('click', () => {
    if (hoverIndex >= 0 && points[hoverIndex] && onOpenEntry) {
      onOpenEntry(points[hoverIndex].entry);
    }
  });

  const kbNav = attachKeyboardNav(host.canvas, {
    getPoints: () => points,
    onSelect: (i) => {
      hoverIndex = i;
      scheduleRender();
      showTooltipFor(i, true);
    },
    onClear: () => {
      hoverIndex = -1;
      host.tooltip.hide();
      scheduleRender();
    },
  });

  host.canvas.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' && hoverIndex >= 0 && points[hoverIndex] && onOpenEntry) {
      ev.preventDefault();
      onOpenEntry(points[hoverIndex].entry);
    }
  });

  const stopResize = observeResize(host.wrap, () => render());

  // ----- public API -----
  function update(nextEntries) {
    entries = nextEntries.filter(e =>
      typeof e.active_params_b === 'number' && typeof e.vibe_quality_score === 'number');
    frontier = computePareto(entries);
    hoverIndex = -1;
    kbNav.reset();
    host.tooltip.hide();
    host.canvas.setAttribute('aria-label',
      `Pareto scatter plot: ${entries.length} runs, vibe quality score versus active parameters. ${frontier.length} Pareto-optimal. Use arrow keys to explore points, Enter to open details.`);
    render();
  }

  return {
    update,
    refresh: render,
    destroy() { stopResize(); },
  };
}