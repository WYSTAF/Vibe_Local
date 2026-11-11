/**
 * Shared canvas chart infrastructure: HiDPI setup, tooltips (textContent only),
 * debounced resize observation, PNG export, keyboard point navigation.
 */
import { el, clear, debounce } from './dom.js';

export const SERIES_COLORS = [
  '#7C5CFF', '#22D3EE', '#F5B84C', '#3DDC97', '#F471B5',
  '#60A5FA', '#A3E635', '#F87171', '#C084FC', '#2DD4BF',
  '#FB923C', '#E879F9',
];

export const FAMILY_COLORS = { moe: '#7C5CFF', dense: '#22D3EE' };

export function hexToRgba(hex, alpha) {
  const h = hex.replace('#', '');
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Canvas + tooltip + screen-reader live region host.
 * @returns {{wrap:HTMLElement, canvas:HTMLCanvasElement, live:HTMLElement, tooltip:object}}
 */
export function createCanvasHost(ariaLabel) {
  const canvas = el('canvas', {
    class: 'chart-canvas',
    tabindex: '0',
    role: 'img',
    'aria-label': ariaLabel,
  });
  const live = el('div', { class: 'visually-hidden', 'aria-live': 'polite', role: 'status' });
  const wrap = el('div', { class: 'chart-wrap' }, canvas, live);
  const tooltip = createTooltip(wrap);
  return { wrap, canvas, live, tooltip };
}

/** Size the canvas backing store to devicePixelRatio; return CSS-pixel context. */
export function fitCanvas(canvas) {
  const dpr = Math.max(1, Math.min(3, window.devicePixelRatio || 1));
  const w = canvas.clientWidth || canvas.parentElement.clientWidth || 600;
  const h = canvas.clientHeight || 420;
  const bw = Math.round(w * dpr);
  const bh = Math.round(h * dpr);
  if (canvas.width !== bw || canvas.height !== bh) {
    canvas.width = bw;
    canvas.height = bh;
  }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h, dpr };
}

/** Positioned tooltip div; all content via textContent. */
export function createTooltip(wrap) {
  const node = el('div', { class: 'chart-tooltip', 'aria-hidden': 'true' });
  wrap.appendChild(node);

  function show(px, py, title, sub, rows) {
    clear(node);
    node.appendChild(el('div', { class: 'tt-title', text: title }));
    if (sub) node.appendChild(el('div', { class: 'tt-sub', text: sub }));
    for (const row of rows || []) {
      node.appendChild(el('div', { class: 'tt-row' },
        el('span', { class: 'k', text: row.k }),
        el('span', { class: `v${row.cls ? ' ' + row.cls : ''}`, text: row.v })));
    }
    node.classList.add('visible');
    // Position after content exists so offsetWidth/Height are correct.
    const ww = wrap.clientWidth;
    const wh = wrap.clientHeight;
    const tw = node.offsetWidth;
    const th = node.offsetHeight;
    let left = px + 16;
    let top = py + 16;
    if (left + tw > ww - 8) left = px - tw - 16;
    if (left < 8) left = 8;
    if (top + th > wh - 8) top = py - th - 16;
    if (top < 8) top = 8;
    node.style.left = `${Math.round(left)}px`;
    node.style.top = `${Math.round(top)}px`;
  }

  function hide() {
    node.classList.remove('visible');
  }

  return { show, hide, node };
}

/** Debounced ResizeObserver (falls back to window resize). */
export function observeResize(target, callback) {
  const debounced = debounce(callback, 130);
  if (typeof ResizeObserver !== 'undefined') {
    const ro = new ResizeObserver(() => debounced());
    ro.observe(target);
    return () => ro.disconnect();
  }
  const handler = () => debounced();
  window.addEventListener('resize', handler);
  return () => window.removeEventListener('resize', handler);
}

/** Export the current canvas bitmap as a PNG download. */
export function exportCanvasPng(canvas, filename) {
  const trigger = (url) => {
    const a = el('a', { href: url, download: filename });
    document.body.appendChild(a);
    a.click();
    a.remove();
  };
  if (typeof canvas.toBlob === 'function') {
    canvas.toBlob((blob) => {
      if (!blob) {
        trigger(canvas.toDataURL('image/png'));
        return;
      }
      const url = URL.createObjectURL(blob);
      trigger(url);
      setTimeout(() => URL.revokeObjectURL(url), 4000);
    }, 'image/png');
  } else {
    trigger(canvas.toDataURL('image/png'));
  }
}

/** Nearest point within maxDist (CSS px). points: [{x, y, ...}] */
export function nearestPoint(points, x, y, maxDist = 18) {
  let best = -1;
  let bestD = maxDist * maxDist;
  for (let i = 0; i < points.length; i++) {
    const dx = points[i].x - x;
    const dy = points[i].y - y;
    const d = dx * dx + dy * dy;
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  }
  return best;
}

/**
 * Keyboard navigation over chart points.
 * getPoints() → [{x, y, announce}] in CSS px; onSelect(i) shows tooltip/highlight.
 */
export function attachKeyboardNav(canvas, { getPoints, onSelect, onClear }) {
  let index = -1;

  canvas.addEventListener('focus', () => {
    const pts = getPoints();
    if (pts.length && index < 0) {
      index = 0;
      onSelect(index);
    }
  });

  canvas.addEventListener('keydown', (ev) => {
    const pts = getPoints();
    if (!pts.length) return;
    let handled = true;
    switch (ev.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        index = (index + 1) % pts.length;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        index = (index - 1 + pts.length) % pts.length;
        break;
      case 'Home': index = 0; break;
      case 'End': index = pts.length - 1; break;
      case 'Escape':
        index = -1;
        onClear();
        break;
      default:
        handled = false;
    }
    if (handled) {
      ev.preventDefault();
      if (index >= 0) onSelect(index);
    }
  });

  canvas.addEventListener('blur', () => {
    index = -1;
    onClear();
  });

  return {
    reset() { index = -1; },
    setIndex(i) { index = i; },
  };
}

/** Shared axis/grid drawing style. */
export const AXIS = {
  grid: 'rgba(35, 40, 56, 0.85)',
  gridStrong: 'rgba(52, 60, 88, 0.9)',
  text: '#9AA3B8',
  textFaint: '#6B7488',
  font: '11px ui-monospace, "Cascadia Code", Consolas, monospace',
  fontSmall: '10px ui-monospace, "Cascadia Code", Consolas, monospace',
  labelFont: '600 11.5px "Segoe UI", system-ui, sans-serif',
};

export function drawAxisLabel(ctx, text, x, y, { align = 'center', color = AXIS.text, font = AXIS.labelFont } = {}) {
  ctx.save();
  ctx.font = font;
  ctx.fillStyle = color;
  ctx.textAlign = align;
  ctx.textBaseline = 'middle';
  ctx.fillText(text, x, y);
  ctx.restore();
}