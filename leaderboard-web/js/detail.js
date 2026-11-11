/**
 * Row detail drawer: per-pillar metric / n / detail plus throughput.
 * Accessible dialog: focus trap, Escape to close, focus restoration.
 */
import { el, clear, safeLink, isHttpsUrl, provenanceBadge, familyBadge, badge } from './dom.js';
import * as fmt from './format.js';
import { PILLARS, pillarScore } from './data.js';

export function createDetailDrawer(root) {
  let openEntry = null;
  let lastFocused = null;
  let scrim = null;
  let drawer = null;
  let closeTimer = 0;
  let onCloseCb = null;

  function close() {
    if (!drawer) return;
    const d = drawer;
    const s = scrim;
    drawer = null;
    scrim = null;
    openEntry = null;
    d.classList.remove('open');
    if (s) s.classList.remove('open');
    document.body.style.overflow = '';
    clearTimeout(closeTimer);
    closeTimer = setTimeout(() => {
      d.remove();
      if (s) s.remove();
    }, 280);
    document.removeEventListener('keydown', onKeydown, true);
    if (lastFocused && typeof lastFocused.focus === 'function') lastFocused.focus();
    lastFocused = null;
    if (onCloseCb) onCloseCb();
  }

  function onKeydown(ev) {
    if (ev.key === 'Escape') {
      ev.preventDefault();
      close();
      return;
    }
    if (ev.key === 'Tab' && drawer) {
      const focusables = drawer.querySelectorAll('button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])');
      if (!focusables.length) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (ev.shiftKey && document.activeElement === first) {
        ev.preventDefault();
        last.focus();
      } else if (!ev.shiftKey && document.activeElement === last) {
        ev.preventDefault();
        first.focus();
      }
    }
  }

  function kvCard(k, v, cls) {
    return el('div', { class: 'kv-card' },
      el('div', { class: 'k', text: k }),
      el('div', { class: `v${cls ? ' ' + cls : ''}`, text: v }));
  }

  function detailList(detail) {
    const dl = el('dl', { class: 'detail-kv' });
    if (!detail || typeof detail !== 'object') {
      dl.appendChild(el('dt', { text: 'detail' }));
      dl.appendChild(el('dd', { text: fmt.detailValue(detail) }));
      return dl;
    }
    for (const [k, v] of Object.entries(detail)) {
      dl.appendChild(el('dt', { text: k }));
      dl.appendChild(el('dd', { text: fmt.detailValue(v) }));
    }
    return dl;
  }

  function open(entry, ctx, onClose) {
    if (drawer) close();
    onCloseCb = onClose || null;
    openEntry = entry;
    lastFocused = document.activeElement;
    const labels = (ctx && ctx.labels) || {};
    const profiles = (ctx && ctx.profiles) || {};
    const pillarIds = (ctx && ctx.pillars) || PILLARS;
    const prof = profiles[entry.hardware_profile] || {};
    const tp = entry.throughput || {};

    const titleId = 'drawer-title';
    const closeBtn = el('button', { type: 'button', class: 'drawer-close', 'aria-label': 'Close details', text: '✕' });
    closeBtn.addEventListener('click', close);

    const badges = el('div', { class: 'drawer-badges' },
      familyBadge(entry.family),
      badge('pareto', entry.quant || 'unknown quant'),
      provenanceBadge(entry.provenance),
      entry.pareto_optimal ? badge('pareto', 'Pareto optimal') : null);

    const head = el('div', { class: 'drawer-head' },
      el('div', { class: 'drawer-head-text' },
        el('h2', { class: 'drawer-title', id: titleId, text: entry.model_id || 'Unknown model' }),
        el('p', { class: 'drawer-sub', text: `${entry.filename || ''}${entry.hf_repo ? ' · ' + entry.hf_repo : ''}` }),
        badges),
      closeBtn);

    const body = el('div', { class: 'drawer-body' });

    // hero scores
    body.appendChild(el('div', { class: 'drawer-section' },
      el('div', { class: 'hero-score' },
        el('div', { class: 'hero-box violet' },
          el('div', { class: 'k', text: 'Vibe quality score' }),
          el('div', { class: 'v', text: fmt.score(entry.vibe_quality_score) })),
        el('div', { class: 'hero-box cyan' },
          el('div', { class: 'k', text: 'Power / parameter' }),
          el('div', { class: 'v', text: fmt.score(entry.power_per_parameter) })),
        el('div', { class: 'hero-box' },
          el('div', { class: 'k', text: 'Decode' }),
          el('div', { class: 'v', text: fmt.tps(tp.decode_tps) }),
          el('div', { class: 'k', text: 'tok/s' })))));

    // model meta
    body.appendChild(el('div', { class: 'drawer-section' },
      el('h3', { text: 'Model & run' }),
      el('div', { class: 'kv-grid' },
        kvCard('Architecture', entry.architecture || '—'),
        kvCard('Family', entry.family === 'moe' ? 'MoE' : 'Dense'),
        kvCard('Total params', fmt.params(entry.total_params_b)),
        kvCard('Active params', fmt.params(entry.active_params_b), 'violet'),
        kvCard('File size', fmt.gb(entry.size_bytes)),
        kvCard('Context', fmt.int(entry.context_length) + ' tok'),
        kvCard('Profile', prof.label || entry.hardware_profile || '—'),
        kvCard('Backend', prof.backend || '—'))));

    // throughput
    body.appendChild(el('div', { class: 'drawer-section' },
      el('h3', { text: 'Throughput & thermals' }),
      el('div', { class: 'kv-grid' },
        kvCard('Prefill', fmt.tps(tp.prefill_tps) + ' t/s', 'cyan'),
        kvCard('Decode', fmt.tps(tp.decode_tps) + ' t/s', 'cyan'),
        kvCard('TTFT', fmt.ms(tp.ttft_ms)),
        kvCard('Peak RAM', fmt.mb(tp.peak_ram_mb)),
        kvCard('Peak VRAM', tp.peak_vram_mb ? fmt.mb(tp.peak_vram_mb) : 'n/a (CPU)'),
        kvCard('Thermal Δ', fmt.celsius(tp.thermal_delta_c), 'amber'))));

    // pillars
    const pillarSection = el('div', { class: 'drawer-section' }, el('h3', { text: '10-pillar breakdown' }));
    for (const pid of pillarIds) {
      const p = (entry.pillars && entry.pillars[pid]) || null;
      const s = pillarScore(entry, pid);
      const block = el('div', { class: 'pillar-detail' });
      block.appendChild(el('div', { class: 'pillar-detail-head' },
        el('span', { class: 'pillar-detail-name', text: labels[pid] || pid }),
        el('span', { class: 'pillar-detail-score', text: fmt.score(s), style: s === null ? {} : { color: s >= 0.75 ? '#93E9C6' : s >= 0.5 ? '#F3D49B' : '#F3B8B8' } })));
      const bar = el('div', { class: 'pillar-bar' },
        el('div', { class: 'pillar-bar-fill', style: { width: `${Math.round((s || 0) * 100)}%` } }));
      block.appendChild(bar);
      if (p) {
        block.appendChild(el('div', { class: 'pillar-detail-meta' },
          el('p', { class: 'pillar-metric' },
            'metric: ', el('code', { text: p.metric || '—' }), ` · n = ${fmt.int(p.n)}`),
          detailList(p.detail)));
      } else {
        block.appendChild(el('div', { class: 'pillar-detail-meta' },
          el('p', { class: 'pillar-metric', text: 'No data for this pillar.' })));
      }
      pillarSection.appendChild(block);
    }
    body.appendChild(pillarSection);

    // actions
    const dl = safeLink(entry.download_url, '⤓ Download GGUF from Hugging Face', 'btn btn-primary');
    const src = safeLink(entry.source_url, 'Source repository ↗', 'btn btn-ghost', isHttpsUrl);
    body.appendChild(el('div', { class: 'drawer-section' },
      el('h3', { text: 'Links' }),
      el('div', { class: 'drawer-actions' }, dl, src),
      el('p', { class: 'panel-desc', text: 'Links open in a new tab. Only https://huggingface.co/ download URLs present in the data are rendered.' })));

    scrim = el('div', { class: 'drawer-scrim' });
    scrim.addEventListener('click', close);
    drawer = el('aside', {
      class: 'drawer',
      role: 'dialog',
      'aria-modal': 'true',
      'aria-labelledby': titleId,
    }, head, body);

    clear(root);
    root.appendChild(scrim);
    root.appendChild(drawer);
    document.body.style.overflow = 'hidden';
    document.addEventListener('keydown', onKeydown, true);
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (scrim) scrim.classList.add('open');
        if (drawer) {
          drawer.classList.add('open');
          closeBtn.focus();
        }
      });
    });
  }

  return {
    open,
    close,
    isOpen: () => drawer !== null,
    currentEntry: () => openEntry,
  };
}