/**
 * View 2 — Quant Degradation Curve (canvas).
 * x = quant ladder Q8_0 → Q2_K, y = vibe_quality_score (or Δ vs best quant).
 * One polyline per model_id for the selected hardware profile.
 * Legend checkboxes toggle models; hover/keyboard tooltips; PNG export.
 */
import { el, clear } from './dom.js';
import * as fmt from './format.js';
import { QUANT_ORDER, groupByModel, quantSeries, quantIndex } from './data.js';
import {
  createCanvasHost, fitCanvas, observeResize, exportCanvasPng,
  attachKeyboardNav, hexToRgba, SERIES_COLORS, AXIS, drawAxisLabel,
} from './chart-utils.js';

const MARGIN = { top: 28, right: 158, bottom: 58, left: 62 };

export function createDegradationView(container) {
  const opts = { mode: 'absolute', labels: true };
  const hidden = new Set();
  let series = [];        // [{modelId, color, points:[{quant, score, delta, entries}]}]
  let drawPoints = [];    // hit-test points in CSS px
  let hoverIndex = -1;
  let rafPending = false;
  let colorMap = new Map();

  // ----- toolbar -----
  const modeSelect = el('select', { id: 'degrade-mode', 'aria-label': 'Y axis mode' },
    el('option', { value: 'absolute', text: 'Y: absolute vibe score' }),
    el('option', { value: 'delta', text: 'Y: Δ vs best quant' }));
  const labelsToggle = el('input', { type: 'checkbox', id: 'degrade-labels', checked: true });
  const allBtn = el('button', { type: 'button', class: 'btn btn-sm btn-ghost', text: 'All models' });
  const noneBtn = el('button', { type: 'button', class: 'btn btn-sm btn-ghost', text: 'None' });
  const exportBtn = el('button', { type: 'button', class: 'btn btn-sm btn-ghost', text: '⤓ Export PNG' });

  const toolbar = el('div', { class: 'chart-toolbar', role: 'group', 'aria-label': 'Degradation curve controls' },
    el('span', { class: 'toolbar-title', text: 'Controls' }),
    modeSelect,
    el('label', { class: 'toggle', for: 'degrade-labels' }, labelsToggle, 'Point labels'),
    allBtn, noneBtn,
    el('span', { class: 'toolbar-spacer' }),
    exportBtn);

  modeSelect.addEventListener('change', () => { opts.mode = modeSelect.value; render(); });
  labelsToggle.addEventListener('change', () => { opts.labels = labelsToggle.checked; render(); });
  allBtn.addEventListener('click', () => { hidden.clear(); syncLegend(); render(); });
  noneBtn.addEventListener('click', () => {
    hidden.clear();
    for (const s of series) hidden.add(s.modelId);
    syncLegend(); render();
  });
  exportBtn.addEventListener('click', () => exportCanvasPng(host.canvas, 'vibe-local-quant-degradation.png'));

  // ----- host -----
  const host = createCanvasHost('Quant degradation curves: vibe quality score across quantization levels for each model. Use arrow keys to explore points.');
  const emptyMsg = el('div', { class: 'chart-empty', hidden: true },
    el('p', { text: 'No multi-quant data for this hardware profile.' }));
  host.wrap.appendChild(emptyMsg);

  const legendBox = el('div', { class: 'model-legend', role: 'group', 'aria-label': 'Toggle models on the degradation chart' });
  const annotation = el('div', { class: 'chart-annotation' });

  container.appendChild(el('div', { class: 'panel-head' },
    el('div', {},
      el('h2', { class: 'panel-title', text: 'Quant Degradation — ' }, el('span', { class: 'accent', text: 'Q8_0 → Q2_K' })),
      el('p', { class: 'panel-desc', text: 'How much quality survives each quantization step on the selected profile. Toggle models in the legend; switch to Δ mode to compare retention curves directly. Values are averaged when a quant was run more than once.' })),
  ));
  container.appendChild(toolbar);
  container.appendChild(host.wrap);
  container.appendChild(legendBox);
  container.appendChild(annotation);

  // ----- series building -----
  function buildSeries(entries) {
    const groups = groupByModel(entries);
    const ids = [...groups.keys()].sort();
    colorMap = new Map(ids.map((id, i) => [id, SERIES_COLORS[i % SERIES_COLORS.length]]));
    const out = [];
    for (const id of ids) {
      const qs = quantSeries(groups.get(id));
      if (!qs.length) continue;
      const baseline = qs[0].score; // leftmost (best) available quant
      out.push({
        modelId: id,
        color: colorMap.get(id),
        points: qs.map(q => ({ ...q, delta: q.score - baseline })),
      });
    }
    return out;
  }

  function buildLegend() {
    clear(legendBox);
    for (const s of series) {
      const cb = el('input', {
        type: 'checkbox',
        checked: !hidden.has(s.modelId),
        'aria-label': `Show ${s.modelId} on degradation chart`,
      });
      const label = el('label', { class: 'model-toggle' + (hidden.has(s.modelId) ? ' off' : '') },
        cb,
        el('span', { class: 'swatch', style: { background: s.color } }),
        el('span', { class: 'm-name', text: s.modelId, title: s.modelId }),
        el('span', { class: 'm-count', text: `${s.points.length}q` }));
      cb.addEventListener('change', () => {
        if (cb.checked) hidden.delete(s.modelId); else hidden.add(s.modelId);
        label.classList.toggle('off', !cb.checked);
        render();
      });
      legendBox.appendChild(label);
    }
  }

  function syncLegend() {
    const boxes = legendBox.querySelectorAll('input[type="checkbox"]');
    series.forEach((s, i) => {
      if (!boxes[i]) return;
      boxes[i].checked = !hidden.has(s.modelId);
      boxes[i].closest('.model-toggle').classList.toggle('off', hidden.has(s.modelId));
    });
  }

  // ----- render -----
  function yValue(p) { return opts.mode === 'delta' ? p.delta : p.score; }

  function render() {
    if (!host.wrap.clientWidth) return;
    const { ctx, w, h } = fitCanvas(host.canvas);
    drawPoints = [];
    hoverIndex = -1;
    host.tooltip.hide();

    const visible = series.filter(s => !hidden.has(s.modelId));
    const allPts = visible.flatMap(s => s.points);
    emptyMsg.hidden = allPts.length > 0;
    if (!allPts.length) {
      clear(annotation);
      annotation.appendChild(document.createTextNode('All models are hidden or no data is available for this profile.'));
      return;
    }

    const narrow = w < 720;
    const right = narrow ? 26 : MARGIN.right;
    const plotW = w - MARGIN.left - right;
    const plotH = h - MARGIN.top - MARGIN.bottom;
    const slotW = plotW / QUANT_ORDER.length;
    const xFor = (quant) => MARGIN.left + (quantIndex(quant) + 0.5) * slotW;

    const vals = allPts.map(yValue);
    let yMin = Math.min(...vals);
    let yMax = Math.max(...vals);
    if (opts.mode === 'delta') {
      yMin = Math.min(yMin - 0.015, -0.01);
      yMax = Math.max(yMax + 0.015, 0.01);
    } else {
      yMin = Math.max(0, yMin - 0.05);
      yMax = Math.min(1.0, yMax + 0.05);
    }
    const yFor = (v) => MARGIN.top + (1 - (v - yMin) / (yMax - yMin || 1)) * plotH;

    // horizontal grid + y ticks
    const step = niceStep((yMax - yMin) / 5);
    ctx.save();
    ctx.font = AXIS.font;
    ctx.lineWidth = 1;
    for (let v = Math.ceil(yMin / step) * step; v <= yMax + 1e-9; v += step) {
      const y = Math.round(yFor(v)) + 0.5;
      const isZero = opts.mode === 'delta' && Math.abs(v) < 1e-9;
      ctx.strokeStyle = isZero ? 'rgba(230, 233, 242, 0.4)' : AXIS.grid;
      ctx.beginPath();
      ctx.moveTo(MARGIN.left, y);
      ctx.lineTo(MARGIN.left + plotW, y);
      ctx.stroke();
      ctx.fillStyle = AXIS.text;
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(opts.mode === 'delta' ? fmt.signed(v, 2) : v.toFixed(2), MARGIN.left - 9, y);
    }
    // vertical slot separators + quant labels
    QUANT_ORDER.forEach((q, i) => {
      const x = Math.round(MARGIN.left + i * slotW) + 0.5;
      ctx.strokeStyle = AXIS.grid;
      ctx.beginPath();
      ctx.moveTo(x, MARGIN.top);
      ctx.lineTo(x, MARGIN.top + plotH);
      ctx.stroke();
      const cx = xFor(q);
      ctx.fillStyle = i === 0 ? '#93E9C6' : i === QUANT_ORDER.length - 1 ? '#F3B8B8' : AXIS.text;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      if (slotW < 58) {
        ctx.save();
        ctx.translate(cx, MARGIN.top + plotH + 10);
        ctx.rotate(-Math.PI / 5);
        ctx.textAlign = 'right';
        ctx.fillText(q, 0, 0);
        ctx.restore();
      } else {
        ctx.fillText(q, cx, MARGIN.top + plotH + 10);
      }
    });
    ctx.strokeStyle = AXIS.gridStrong;
    ctx.strokeRect(MARGIN.left + 0.5, MARGIN.top + 0.5, plotW, plotH);
    ctx.restore();

    drawAxisLabel(ctx, 'Quantization level (higher → more compression)', MARGIN.left + plotW / 2, h - (narrow ? 8 : 12));
    ctx.save();
    ctx.translate(16, MARGIN.top + plotH / 2);
    ctx.rotate(-Math.PI / 2);
    drawAxisLabel(ctx, opts.mode === 'delta' ? 'Δ vibe score vs best quant' : 'Vibe quality score', 0, 0);
    ctx.restore();

    // polylines
    const labelSlots = new Map(); // quantIndex -> [{x, y, text, color}]
    const endLabels = [];         // [{x, y, text, color}]
    for (const s of visible) {
      ctx.save();
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 2;
      ctx.lineJoin = 'round';
      ctx.beginPath();
      s.points.forEach((p, i) => {
        const x = xFor(p.quant);
        const y = yFor(yValue(p));
        if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      });
      ctx.stroke();

      // points + label candidates
      for (const p of s.points) {
        const x = xFor(p.quant);
        const y = yFor(yValue(p));
        drawPoints.push({ x, y, series: s, point: p });
        ctx.beginPath();
        ctx.arc(x, y, 3.6, 0, Math.PI * 2);
        ctx.fillStyle = '#0D0F17';
        ctx.fill();
        ctx.lineWidth = 2;
        ctx.strokeStyle = s.color;
        ctx.stroke();
        if (opts.labels) {
          const slot = quantIndex(p.quant);
          if (!labelSlots.has(slot)) labelSlots.set(slot, []);
          labelSlots.get(slot).push({
            x, y,
            text: opts.mode === 'delta' ? fmt.signed(p.delta, 2) : p.score.toFixed(2),
            color: s.color,
          });
        }
      }

      // end-of-line model label candidate (desktop only)
      if (!narrow && s.points.length) {
        const last = s.points[s.points.length - 1];
        endLabels.push({ x: xFor(last.quant), y: yFor(yValue(last)), text: s.modelId, color: s.color });
      }
      ctx.restore();
    }

    // point labels with per-slot collision avoidance (hover always shows exact value)
    if (opts.labels) {
      ctx.save();
      ctx.font = AXIS.fontSmall;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'bottom';
      for (const candidates of labelSlots.values()) {
        candidates.sort((a, b) => a.y - b.y);
        let lastY = -Infinity;
        for (const c of candidates) {
          if (c.y - lastY < 10) continue; // too close to the label above — skip
          lastY = c.y;
          ctx.fillStyle = hexToRgba(c.color, 0.95);
          ctx.fillText(c.text, c.x, c.y - 7);
        }
      }
      ctx.restore();
    }

    // end labels: de-collide vertically, draw leader lines when displaced
    if (endLabels.length) {
      ctx.save();
      ctx.font = AXIS.fontSmall;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const maxChars = Math.floor((right - 16) / 6.2);
      endLabels.sort((a, b) => a.y - b.y);
      const minGap = 13;
      const top = MARGIN.top + 6;
      const bottom = MARGIN.top + plotH - 6;
      for (let i = 0; i < endLabels.length; i++) {
        const prev = i > 0 ? endLabels[i - 1].ly : top - minGap;
        endLabels[i].ly = Math.max(endLabels[i].y, prev + minGap);
      }
      const overflow = endLabels[endLabels.length - 1].ly - bottom;
      if (overflow > 0) {
        for (const l of endLabels) l.ly -= overflow;
        for (let i = endLabels.length - 2; i >= 0; i--) {
          endLabels[i].ly = Math.min(endLabels[i].ly, endLabels[i + 1].ly - minGap);
        }
      }
      for (const l of endLabels) {
        let name = l.text;
        if (name.length > maxChars) name = name.slice(0, Math.max(4, maxChars - 1)) + '…';
        if (Math.abs(l.ly - l.y) > 3) {
          ctx.strokeStyle = hexToRgba(l.color, 0.5);
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(l.x + 5, l.y);
          ctx.lineTo(l.x + 9, l.ly);
          ctx.stroke();
        }
        ctx.fillStyle = l.color;
        ctx.fillText(name, l.x + 11, l.ly);
      }
      ctx.restore();
    }

    // hover highlight
    if (hoverIndex >= 0 && hoverIndex < drawPoints.length) {
      const p = drawPoints[hoverIndex];
      ctx.save();
      ctx.beginPath();
      ctx.arc(p.x, p.y, 7.5, 0, Math.PI * 2);
      ctx.strokeStyle = '#E6E9F2';
      ctx.lineWidth = 1.6;
      ctx.stroke();
      ctx.restore();
    }

    updateAnnotation(visible);
  }

  function niceStep(raw) {
    const mag = Math.pow(10, Math.floor(Math.log10(raw || 1)));
    const norm = raw / mag;
    if (norm <= 1) return mag;
    if (norm <= 2) return 2 * mag;
    if (norm <= 5) return 5 * mag;
    return 10 * mag;
  }

  function updateAnnotation(visible) {
    clear(annotation);
    const withBoth = visible
      .map(s => {
        const q8 = s.points.find(p => p.quant === 'Q8_0');
        const q2 = s.points.find(p => p.quant === 'Q2_K');
        return q8 && q2 ? { id: s.modelId, drop: q8.score - q2.score, rel: q2.score / q8.score } : null;
      })
      .filter(Boolean);
    if (withBoth.length) {
      const avgDrop = withBoth.reduce((a, b) => a + b.drop, 0) / withBoth.length;
      const worst = withBoth.slice().sort((a, b) => b.drop - a.drop)[0];
      const best = withBoth.slice().sort((a, b) => a.drop - b.drop)[0];
      annotation.appendChild(el('strong', { text: 'Q2_K cliff: ' }));
      annotation.appendChild(document.createTextNode(
        `across ${withBoth.length} models with both endpoints, Q2_K loses ${fmt.score(avgDrop)} vibe score on average (${fmt.pct(avgDrop / 0.75, 0)} of a typical 0.75 baseline). `));
      annotation.appendChild(el('strong', { text: 'Most fragile: ' }));
      annotation.appendChild(document.createTextNode(`${worst.id} (−${fmt.score(worst.drop)}). `));
      annotation.appendChild(el('strong', { text: 'Most robust: ' }));
      annotation.appendChild(document.createTextNode(`${best.id} (−${fmt.score(best.drop)}). Q4_K_M / IQ4_XS usually keep ≥97% of Q8_0 quality — the sweet spot for 8 GB cards.`));
    } else {
      annotation.appendChild(document.createTextNode(
        'Tip: profiles with a full quant ladder (e.g. Tesla T4) show the complete Q8_0 → Q2_K story. Δ mode normalizes each model to its best available quant.'));
    }
  }

  function scheduleRender() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => { rafPending = false; render(); });
  }

  // ----- interaction -----
  function showTooltipFor(index, announce) {
    const dp = drawPoints[index];
    if (!dp) return;
    const p = dp.point;
    const s = dp.series;
    const decodeVals = p.entries.map(e => e.throughput && e.throughput.decode_tps).filter(v => typeof v === 'number');
    const avgDecode = decodeVals.length ? decodeVals.reduce((a, b) => a + b, 0) / decodeVals.length : null;
    host.tooltip.show(dp.x, dp.y, s.modelId, `${p.quant} · ${p.entries.length} run${p.entries.length > 1 ? 's' : ''} averaged`, [
      { k: 'Vibe score', v: fmt.score(p.score), cls: 'cyan' },
      { k: 'Δ vs best quant', v: fmt.signed(p.delta), cls: p.delta < -0.05 ? 'amber' : 'green' },
      { k: 'Retention', v: fmt.pct(p.score / (p.score - p.delta || 1), 1) },
      { k: 'Avg decode', v: avgDecode === null ? '—' : `${fmt.tps(avgDecode)} tok/s` },
      { k: 'Provenance', v: [...new Set(p.entries.map(e => e.provenance))].join(', ') },
    ]);
    if (announce) {
      host.live.textContent = `${s.modelId} at ${p.quant}: vibe score ${fmt.score(p.score)}, change versus best quant ${fmt.signed(p.delta)}.`;
    }
  }

  host.canvas.addEventListener('mousemove', (ev) => {
    const rect = host.canvas.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;
    let best = -1;
    let bestD = 16 * 16;
    for (let i = 0; i < drawPoints.length; i++) {
      const dx = drawPoints[i].x - x;
      const dy = drawPoints[i].y - y;
      const d = dx * dx + dy * dy;
      if (d < bestD) { bestD = d; best = i; }
    }
    if (best !== hoverIndex) {
      hoverIndex = best;
      scheduleRender();
    }
    if (best >= 0) showTooltipFor(best, false);
    else host.tooltip.hide();
  });

  host.canvas.addEventListener('mouseleave', () => {
    if (hoverIndex !== -1) { hoverIndex = -1; scheduleRender(); }
    host.tooltip.hide();
  });

  const kbNav = attachKeyboardNav(host.canvas, {
    getPoints: () => drawPoints,
    onSelect: (i) => { hoverIndex = i; scheduleRender(); showTooltipFor(i, true); },
    onClear: () => { hoverIndex = -1; host.tooltip.hide(); scheduleRender(); },
  });

  const stopResize = observeResize(host.wrap, () => render());

  // ----- public API -----
  function update(entries) {
    series = buildSeries(entries);
    // drop hidden ids that no longer exist
    for (const id of [...hidden]) {
      if (!series.some(s => s.modelId === id)) hidden.delete(id);
    }
    buildLegend();
    kbNav.reset();
    host.canvas.setAttribute('aria-label',
      `Quant degradation curves for ${series.length} models across ${QUANT_ORDER.length} quantization levels. Use arrow keys to explore points.`);
    render();
  }

  return { update, refresh: render, destroy() { stopResize(); } };
}