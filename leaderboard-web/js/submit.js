/**
 * View 4 — Submit measured run.
 * Client-side validation, POST /api/leaderboard/submit (JSON), server response
 * display. Disabled with an explanation when running on the bundled catalog.
 */
import { el, clear, isHuggingFaceUrl } from './dom.js';
import * as fmt from './format.js';
import { PILLARS, QUANT_ORDER, QUALITY_WEIGHTS, DEFAULT_PILLAR_LABELS, FALLBACK_PROFILES, vibeQualityScore } from './data.js';
import { submitRun } from './api.js';

const HF_REPO_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const MODEL_ID_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{1,80}$/;

export function createSubmitView(container, { onSuccess }) {
  let mode = 'catalog';
  let labels = { ...DEFAULT_PILLAR_LABELS };
  let profiles = { ...FALLBACK_PROFILES };
  let busy = false;

  const fieldRegistry = [];

  function makeField({ id, label, type = 'text', required = false, hint = '', attrs = {}, options = null, section }) {
    const input = options
      ? el('select', { id, name: id, ...attrs }, ...options.map(o => el('option', { value: o.value, text: o.text })))
      : el('input', { id, name: id, type, ...attrs });
    const errId = `${id}-error`;
    const errNode = el('p', { class: 'field-error', id: errId, role: 'alert' });
    const labelNode = el('label', { for: id },
      label,
      required ? el('span', { class: 'req', 'aria-hidden': 'true', text: ' *' }) : null,
      hint ? el('span', { class: 'hint', text: hint }) : null);
    input.setAttribute('aria-describedby', errId);
    if (required) input.setAttribute('aria-required', 'true');
    const wrap = el('div', { class: 'field' }, labelNode, input, errNode);
    const field = { id, input, errNode, wrap, required, type, section };
    fieldRegistry.push(field);
    input.addEventListener('input', () => {
      if (input.getAttribute('aria-invalid') === 'true') validateField(field);
      updatePreview();
    });
    input.addEventListener('change', () => updatePreview());
    return field;
  }

  function setError(field, message) {
    field.errNode.textContent = message || '';
    if (message) field.input.setAttribute('aria-invalid', 'true');
    else field.input.removeAttribute('aria-invalid');
  }

  // ----- identity fields -----
  const fModel = makeField({ id: 'sub-model', label: 'Model ID', required: true, hint: 'e.g. qwen3-8b', attrs: { placeholder: 'qwen3-8b', autocomplete: 'off', maxlength: '80' } });
  const fRepo = makeField({ id: 'sub-repo', label: 'HF repo', required: true, hint: 'org/name', attrs: { placeholder: 'Qwen/Qwen3-8B-GGUF', autocomplete: 'off' } });
  const fFile = makeField({ id: 'sub-file', label: 'Filename', required: true, hint: '.gguf', attrs: { placeholder: 'qwen3-8b-Q4_K_M.gguf', autocomplete: 'off' } });
  const fUrl = makeField({ id: 'sub-url', label: 'Download URL', hint: 'optional · huggingface.co only', attrs: { placeholder: 'https://huggingface.co/…', autocomplete: 'off', inputmode: 'url' } });
  const fQuant = makeField({ id: 'sub-quant', label: 'Quant', required: true, options: QUANT_ORDER.map(q => ({ value: q, text: q })) });
  const fArch = makeField({ id: 'sub-arch', label: 'Architecture', hint: 'e.g. qwen3, llama', attrs: { placeholder: 'qwen3', autocomplete: 'off' } });
  const fFamily = makeField({ id: 'sub-family', label: 'Family', required: true, options: [{ value: 'dense', text: 'Dense' }, { value: 'moe', text: 'MoE' }] });
  const fTotal = makeField({ id: 'sub-total', label: 'Total params (B)', required: true, type: 'number', attrs: { min: '0.01', max: '2000', step: '0.01', placeholder: '8.19' } });
  const fActive = makeField({ id: 'sub-active', label: 'Active params (B)', required: true, type: 'number', hint: '= total for dense', attrs: { min: '0.01', max: '2000', step: '0.01', placeholder: '8.19' } });
  const fSize = makeField({ id: 'sub-size', label: 'File size (GB)', type: 'number', hint: 'optional', attrs: { min: '0.01', max: '2000', step: '0.01', placeholder: '4.68' } });
  const fCtx = makeField({ id: 'sub-ctx', label: 'Context length', type: 'number', hint: 'tokens · optional', attrs: { min: '128', max: '2000000', step: '1', placeholder: '32768' } });
  const fProfile = makeField({ id: 'sub-profile', label: 'Hardware profile', required: true, options: Object.entries(FALLBACK_PROFILES).map(([id, p]) => ({ value: id, text: p.label })) });

  // ----- pillar fields -----
  const pillarFields = PILLARS.map(pid => {
    const weight = QUALITY_WEIGHTS[pid];
    const input = el('input', {
      id: `sub-pillar-${pid}`, name: `pillar-${pid}`, type: 'number',
      min: '0', max: '1', step: '0.001', placeholder: '0.000–1.000',
      'aria-required': 'true', 'aria-describedby': `sub-pillar-${pid}-error`,
    });
    const errNode = el('p', { class: 'field-error', id: `sub-pillar-${pid}-error`, role: 'alert' });
    const wrap = el('div', { class: 'field pillar-field' },
      el('label', { for: `sub-pillar-${pid}` },
        labels[pid] || pid,
        weight ? el('span', { class: 'weight-tag', text: `w=${weight.toFixed(2)}` }) : el('span', { class: 'weight-tag', text: 'hw pillar' })),
      input, errNode);
    const field = { id: `sub-pillar-${pid}`, input, errNode, wrap, required: true, type: 'number', pillar: pid };
    fieldRegistry.push(field);
    input.addEventListener('input', () => {
      if (input.getAttribute('aria-invalid') === 'true') validateField(field);
      updatePreview();
    });
    return field;
  });

  // ----- throughput fields -----
  const fPrefill = makeField({ id: 'sub-prefill', label: 'Prefill (tok/s)', required: true, type: 'number', attrs: { min: '0.01', step: '0.1', placeholder: '850' } });
  const fDecode = makeField({ id: 'sub-decode', label: 'Decode (tok/s)', required: true, type: 'number', attrs: { min: '0.01', step: '0.1', placeholder: '42.5' } });
  const fTtft = makeField({ id: 'sub-ttft', label: 'TTFT (ms)', required: true, type: 'number', attrs: { min: '0', step: '1', placeholder: '310' } });
  const fRam = makeField({ id: 'sub-ram', label: 'Peak RAM (MB)', type: 'number', hint: 'optional', attrs: { min: '0', step: '1', placeholder: '6200' } });
  const fVram = makeField({ id: 'sub-vram', label: 'Peak VRAM (MB)', type: 'number', hint: 'optional', attrs: { min: '0', step: '1', placeholder: '5900' } });
  const fThermal = makeField({ id: 'sub-thermal', label: 'Thermal Δ (°C)', type: 'number', hint: 'optional', attrs: { step: '0.1', placeholder: '18.5' } });

  // ----- live preview -----
  const previewVibe = el('span', { class: 'v', text: '—' });
  const previewPower = el('span', { class: 'v', text: '—' });
  const preview = el('div', { class: 'hero-score', 'aria-live': 'polite', 'aria-label': 'Live preview of derived scores' },
    el('div', { class: 'hero-box violet' }, el('div', { class: 'k', text: 'Preview · vibe score' }), previewVibe),
    el('div', { class: 'hero-box cyan' }, el('div', { class: 'k', text: 'Preview · power/param' }), previewPower));

  function updatePreview() {
    const pillars = {};
    for (const f of pillarFields) {
      const v = parseFloat(f.input.value);
      if (Number.isFinite(v) && v >= 0 && v <= 1) pillars[f.pillar] = { score: v };
    }
    const fake = { pillars, active_params_b: parseFloat(fActive.input.value) };
    const vq = vibeQualityScore(fake);
    previewVibe.textContent = fmt.score(vq);
    previewPower.textContent = fmt.score(vq !== null && Number.isFinite(fake.active_params_b) && fake.active_params_b > 0 ? vq / fake.active_params_b : null);
  }

  // ----- validation -----
  function validateField(field) {
    const raw = field.input.value.trim();
    const id = field.id;
    let msg = '';

    if (field.required && raw === '') msg = 'Required.';
    else if (raw !== '') {
      if (id === 'sub-model' && !MODEL_ID_RE.test(raw)) msg = 'Use letters, digits, dots, dashes, underscores (2–80 chars).';
      else if (id === 'sub-repo' && !HF_REPO_RE.test(raw)) msg = 'Expected org/name, e.g. Qwen/Qwen3-8B-GGUF.';
      else if (id === 'sub-file' && (!/\.gguf$/i.test(raw) || /[\\/]/.test(raw))) msg = 'Must be a .gguf filename with no path separators.';
      else if (id === 'sub-url' && !isHuggingFaceUrl(raw)) msg = 'Only https://huggingface.co/ URLs are accepted.';
      else if (field.type === 'number') {
        const v = Number(raw);
        if (!Number.isFinite(v)) msg = 'Must be a number.';
        else if (field.pillar && (v < 0 || v > 1)) msg = 'Score must be between 0 and 1.';
        else if (field.input.min !== '' && v < Number(field.input.min)) msg = `Must be ≥ ${field.input.min}.`;
        else if (field.input.max !== '' && v > Number(field.input.max)) msg = `Must be ≤ ${field.input.max}.`;
      }
    }
    // cross-field: active ≤ total
    if (!msg && id === 'sub-active') {
      const total = Number(fTotal.input.value);
      const active = Number(raw);
      if (Number.isFinite(total) && Number.isFinite(active) && active > total) msg = 'Active params cannot exceed total params.';
    }
    setError(field, msg);
    return !msg;
  }

  function validateAll() {
    let firstBad = null;
    const messages = [];
    for (const field of fieldRegistry) {
      const ok = validateField(field);
      if (!ok) {
        messages.push(`${labelText(field)}: ${field.errNode.textContent}`);
        if (!firstBad) firstBad = field.input;
      }
    }
    return { ok: messages.length === 0, messages, firstBad };
  }

  function labelText(field) {
    const lab = field.wrap.querySelector('label');
    return lab ? lab.textContent.replace(/\s*\*\s*$/, '').trim() : field.id;
  }

  // ----- payload -----
  function buildPayload() {
    const numOrUndef = (f) => {
      const raw = f.input.value.trim();
      if (raw === '') return undefined;
      const v = Number(raw);
      return Number.isFinite(v) ? v : undefined;
    };
    const repo = fRepo.input.value.trim();
    const filename = fFile.input.value.trim();
    const urlRaw = fUrl.input.value.trim();
    const pillars = {};
    for (const f of pillarFields) {
      pillars[f.pillar] = { score: Number(f.input.value) };
    }
    const throughput = {
      prefill_tps: Number(fPrefill.input.value),
      decode_tps: Number(fDecode.input.value),
      ttft_ms: Number(fTtft.input.value),
    };
    const ram = numOrUndef(fRam); if (ram !== undefined) throughput.peak_ram_mb = ram;
    const vram = numOrUndef(fVram); if (vram !== undefined) throughput.peak_vram_mb = vram;
    const thermal = numOrUndef(fThermal); if (thermal !== undefined) throughput.thermal_delta_c = thermal;

    const payload = {
      model_id: fModel.input.value.trim(),
      hf_repo: repo,
      filename,
      download_url: isHuggingFaceUrl(urlRaw) ? urlRaw : `https://huggingface.co/${repo}/resolve/main/${filename}`,
      quant: fQuant.input.value,
      architecture: fArch.input.value.trim() || undefined,
      family: fFamily.input.value,
      total_params_b: Number(fTotal.input.value),
      active_params_b: Number(fActive.input.value),
      context_length: numOrUndef(fCtx),
      hardware_profile: fProfile.input.value,
      provenance: 'submitted',
      pillars,
      throughput,
    };
    const sizeGb = numOrUndef(fSize);
    if (sizeGb !== undefined) payload.size_bytes = Math.round(sizeGb * 1073741824);
    return payload;
  }

  // ----- status / summary regions -----
  const errorSummary = el('div', { class: 'error-summary', role: 'alert', hidden: true });
  const statusBox = el('div', { class: 'form-status', role: 'status', 'aria-live': 'polite', hidden: true });

  function showStatus(ok, title, body) {
    clear(statusBox);
    statusBox.hidden = false;
    statusBox.classList.toggle('ok', ok);
    statusBox.classList.toggle('err', !ok);
    statusBox.appendChild(el('h4', { text: title }));
    if (body !== null && body !== undefined) {
      const text = typeof body === 'string' ? body : JSON.stringify(body, null, 2);
      statusBox.appendChild(el('pre', { text }));
    }
    statusBox.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  // ----- form assembly -----
  const notice = el('div', { class: 'submit-notice' },
    el('span', { class: 'notice-icon', 'aria-hidden': 'true', text: '⚠' }),
    el('span', { text: '' }));
  const noticeText = notice.lastChild;

  const submitBtn = el('button', { type: 'submit', class: 'btn btn-primary', text: 'Submit measured run' });
  const sampleBtn = el('button', { type: 'button', class: 'btn btn-ghost', text: 'Fill sample run' });
  const resetBtn = el('button', { type: 'button', class: 'btn btn-ghost', text: 'Reset form' });

  const fieldset = el('fieldset', {},
    el('div', { class: 'form-section' },
      el('h3', { text: 'Model identity' }),
      el('div', { class: 'form-grid' }, fModel.wrap, fRepo.wrap, fFile.wrap, fUrl.wrap, fQuant.wrap, fArch.wrap, fFamily.wrap)),
    el('div', { class: 'form-section' },
      el('h3', { text: 'Parameters & run' }),
      el('div', { class: 'form-grid' }, fTotal.wrap, fActive.wrap, fSize.wrap, fCtx.wrap, fProfile.wrap)),
    el('div', { class: 'form-section' },
      el('h3', { text: '10 pillar scores (0–1)' }),
      el('div', { class: 'pillar-inputs' }, ...pillarFields.map(f => f.wrap)),
      el('div', { class: 'form-section' }, preview)),
    el('div', { class: 'form-section' },
      el('h3', { text: 'Throughput & thermals' }),
      el('div', { class: 'form-grid' }, fPrefill.wrap, fDecode.wrap, fTtft.wrap, fRam.wrap, fVram.wrap, fThermal.wrap)),
    el('div', { class: 'form-actions' }, submitBtn, sampleBtn, resetBtn));

  const form = el('form', { novalidate: true, 'aria-label': 'Submit a measured benchmark run' },
    errorSummary, fieldset, statusBox);

  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    if (busy || mode !== 'live') return;
    clear(errorSummary);
    errorSummary.hidden = true;
    const { ok, messages, firstBad } = validateAll();
    if (!ok) {
      errorSummary.hidden = false;
      errorSummary.appendChild(el('strong', { text: `Please fix ${messages.length} field${messages.length > 1 ? 's' : ''}:` }));
      const ul = el('ul');
      for (const m of messages) ul.appendChild(el('li', { text: m }));
      errorSummary.appendChild(ul);
      if (firstBad) firstBad.focus();
      return;
    }
    busy = true;
    submitBtn.disabled = true;
    submitBtn.textContent = 'Submitting…';
    try {
      const payload = buildPayload();
      const res = await submitRun(payload);
      if (res.ok) {
        showStatus(true, `Accepted — HTTP ${res.status}`, res.body);
        if (onSuccess) onSuccess(payload);
      } else {
        showStatus(false, `Rejected — HTTP ${res.status}`, res.body);
      }
    } catch (err) {
      showStatus(false, 'Network error', `${err.message}. The live server must be running to accept submissions.`);
    } finally {
      busy = false;
      submitBtn.disabled = mode !== 'live';
      submitBtn.textContent = 'Submit measured run';
    }
  });

  sampleBtn.addEventListener('click', () => {
    const sample = {
      'sub-model': 'qwen3-8b', 'sub-repo': 'Qwen/Qwen3-8B-GGUF', 'sub-file': 'qwen3-8b-Q4_K_M.gguf',
      'sub-url': 'https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/qwen3-8b-Q4_K_M.gguf',
      'sub-quant': 'Q4_K_M', 'sub-arch': 'qwen3', 'sub-family': 'dense',
      'sub-total': '8.19', 'sub-active': '8.19', 'sub-size': '4.68', 'sub-ctx': '32768',
      'sub-profile': fProfile.input.value || 'rx580_vulkan',
      'sub-prefill': '812.4', 'sub-decode': '41.7', 'sub-ttft': '324',
      'sub-ram': '6480', 'sub-vram': '5860', 'sub-thermal': '19.4',
    };
    const pillarSample = [0.742, 0.698, 0.771, 0.512, 0.758, 0.724, 0.689, 0.961, 0.634, 0.702];
    for (const [id, v] of Object.entries(sample)) {
      const f = fieldRegistry.find(x => x.id === id);
      if (f) { f.input.value = v; setError(f, ''); }
    }
    pillarFields.forEach((f, i) => { f.input.value = String(pillarSample[i]); setError(f, ''); });
    updatePreview();
  });

  resetBtn.addEventListener('click', () => {
    form.reset();
    for (const f of fieldRegistry) setError(f, '');
    errorSummary.hidden = true;
    statusBox.hidden = true;
    updatePreview();
  });

  container.appendChild(el('div', { class: 'panel-head' },
    el('div', {},
      el('h2', { class: 'panel-title', text: 'Submit a ' }, el('span', { class: 'accent', text: 'measured run' })),
      el('p', { class: 'panel-desc', text: 'Ran the 10-pillar suite on your own rig? Submit the numbers. Submissions are stored with provenance “submitted” and flagged for review before they can displace measured reference runs.' })),
  ));
  container.appendChild(notice);
  container.appendChild(form);

  // ----- mode / context -----
  function setMode(nextMode) {
    mode = nextMode;
    const live = mode === 'live';
    fieldset.disabled = !live;
    notice.classList.toggle('live', live);
    notice.querySelector('.notice-icon').textContent = live ? '✓' : '⚠';
    noticeText.textContent = live
      ? 'Live server connected — submissions POST to /api/leaderboard/submit as JSON.'
      : 'Submissions are disabled in bundled-catalog mode. Start the Vibe Local server (POST /api/leaderboard/submit) to submit measured runs; the offline catalog is read-only.';
  }

  function setContext(nextLabels, nextProfiles) {
    if (nextLabels) labels = { ...DEFAULT_PILLAR_LABELS, ...nextLabels };
    if (nextProfiles) profiles = nextProfiles;
    // refresh pillar labels
    for (const f of pillarFields) {
      const lab = f.wrap.querySelector('label');
      if (lab && lab.firstChild) lab.firstChild.textContent = labels[f.pillar] || f.pillar;
    }
    // refresh profile options, preserving selection
    const current = fProfile.input.value;
    clear(fProfile.input);
    for (const [id, p] of Object.entries(profiles)) {
      fProfile.input.appendChild(el('option', { value: id, text: p.label || id }));
    }
    if (profiles[current]) fProfile.input.value = current;
  }

  setMode('catalog');
  updatePreview();

  return { setMode, setContext, destroy() {} };
}