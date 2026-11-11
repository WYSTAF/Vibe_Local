/**
 * DOM construction helpers.
 * Security: every node is built with document.createElement + textContent.
 * No innerHTML / insertAdjacentHTML / document.write / inline handlers anywhere.
 * Style objects are applied through the CSSOM (element.style), which is
 * permitted under `style-src 'self'` (unlike inline style attributes in markup).
 */

/**
 * Create an element.
 * @param {string} tag
 * @param {Object} [attrs] attributes; special keys: class, text, dataset, style, value
 * @param {...(Node|string|number|null|undefined|Array)} children
 */
export function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, val] of Object.entries(attrs)) {
    if (val === null || val === undefined || val === false) continue;
    if (key === 'class') node.className = val;
    else if (key === 'text') node.textContent = String(val);
    else if (key === 'dataset') Object.assign(node.dataset, val);
    else if (key === 'style') Object.assign(node.style, val); // CSSOM — CSP-safe
    else if (key === 'value') node.value = val;
    else if (val === true) node.setAttribute(key, '');
    else node.setAttribute(key, String(val));
  }
  append(node, children);
  return node;
}

export function append(node, children) {
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    node.appendChild(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

export function clear(node) {
  node.replaceChildren();
  return node;
}

/** Only https://huggingface.co/ URLs are allowed for model download links. */
export function isHuggingFaceUrl(url) {
  return typeof url === 'string' && url.startsWith('https://huggingface.co/');
}

/** Any absolute https URL (used for secondary source links). */
export function isHttpsUrl(url) {
  if (typeof url !== 'string' || !url.startsWith('https://')) return false;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && u.hostname.length > 0;
  } catch {
    return false;
  }
}

/**
 * Build a validated external link. Returns a plain span when the URL is invalid,
 * so untrusted data can never inject a javascript:/data: href.
 */
export function safeLink(url, label, className, validator = isHuggingFaceUrl) {
  if (!validator(url)) {
    return el('span', { class: 'link-na', text: label === 'HF ↗' ? '—' : 'n/a', title: 'No valid HTTPS link in data' });
  }
  return el('a', {
    class: className,
    href: url,
    target: '_blank',
    rel: 'noopener noreferrer',
    text: label,
  });
}

export function badge(kind, text) {
  return el('span', { class: `badge badge-${kind}`, text });
}

export function provenanceBadge(provenance) {
  switch (provenance) {
    case 'measured': return badge('measured', 'measured');
    case 'submitted': return badge('submitted', 'submitted');
    default: return badge('estimate', 'estimate');
  }
}

export function familyBadge(family) {
  return family === 'moe' ? badge('moe', 'MoE') : badge('dense', 'Dense');
}

/** Debounce helper (trailing edge). */
export function debounce(fn, wait = 120) {
  let t = 0;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), wait);
  };
}