// Combo Editor front end. The server owns all camera/overlay maths (see
// editor/camera.py); this file samples the per-frame arrays it returns so the
// preview matches the export.

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const OUT_W = 1080, OUT_H = 1920, ASPECT = OUT_W / OUT_H;
const PARTS = ['combo', 'startup', 'animation', 'post'];
const PART_LABEL = { combo: 'Combo', startup: 'Super startup', animation: 'Super animation', post: 'Post super' };
const MARKERS = [
  ['in', 'In'], ['superStart', 'Super start'], ['animStart', 'Animation start'], ['postStart', 'Post start'], ['out', 'Out'],
];

const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const COLORS = () => ({ combo: css('--combo'), startup: css('--startup'), animation: css('--animation'), post: css('--post') });

const vClean = $('#vClean'), vUi = $('#vUi'), aMusic = $('#aMusic');
const preview = $('#preview'), pctx = preview.getContext('2d');
const source = $('#source'), sctx = source.getContext('2d');
const tlCanvas = $('#timeline'), tctx = tlCanvas.getContext('2d');

const state = {
  project: null,
  info: { clean: null, ui: null },
  timeline: null,
  track: null,
  waveform: null,
  envelopes: null,
  selected: null,       // selected overlay element id
  tab: 'sources',
  sourceMode: 'clean',
  live: null,           // camera override while dragging
  drag: null,
};

// ---------------------------------------------------------------- helpers

async function api(path, body, method) {
  const opts = { method: method || (body === undefined ? 'GET' : 'POST'), headers: {} };
  if (body !== undefined) { opts.body = JSON.stringify(body); opts.headers['Content-Type'] = 'application/json'; }
  const res = await fetch(path, opts);
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).detail || msg; } catch { /* not JSON */ }
    throw new Error(msg);
  }
  const type = res.headers.get('content-type') || '';
  return type.includes('json') ? res.json() : res.blob();
}

let statusTimer;
function status(msg, isError = false) {
  const el = $('#status');
  el.textContent = msg;
  el.style.color = isError ? '#ff8f8a' : '';
  clearTimeout(statusTimer);
  if (msg) statusTimer = setTimeout(() => (el.textContent = ''), isError ? 8000 : 4000);
}

async function guarded(fn, busyMsg) {
  try {
    if (busyMsg) status(busyMsg);
    return await fn();
  } catch (err) {
    console.error(err);
    status(err.message || String(err), true);
  }
}

const getPath = (obj, path) => path.split('.').reduce((o, k) => (o == null ? o : o[k]), obj);
function setPath(obj, path, value) {
  const keys = path.split('.');
  const last = keys.pop();
  keys.reduce((o, k) => (o[k] ??= {}), obj)[last] = value;
}

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const fps = () => state.info.clean?.fps || 60;
const duration = () => state.info.clean?.duration || 0;
const srcSize = () => [state.info.clean?.width || 1920, state.info.clean?.height || 1080];
const fmtTime = (t) => {
  if (t == null || !isFinite(t)) return '—';
  const m = Math.floor(t / 60);
  return `${m}:${(t - m * 60).toFixed(3).padStart(6, '0')}`;
};
const now = () => vClean.currentTime || 0;
const frameOf = (t) => Math.round(t * fps());
const snap = (t) => frameOf(t) / fps();

function save() {
  try { localStorage.setItem('comboEditor.project', JSON.stringify(state.project)); } catch { /* storage unavailable */ }
}

// Debounced timeline refresh after any edit that affects camera or overlay alphas.
let tlTimer, tlSeq = 0;
function changed({ timeline = true } = {}) {
  save();
  if (!timeline) return;
  clearTimeout(tlTimer);
  tlTimer = setTimeout(refreshTimeline, 120);
}

async function refreshTimeline() {
  if (!state.project.sources.clean || !state.info.clean) return;
  const seq = ++tlSeq;
  try {
    const tl = await api('/api/timeline', state.project);
    if (seq !== tlSeq) return;
    state.timeline = tl;
    state.live = null;
    $('#trackStatus').textContent = tl.tracked ? 'Tracked ✓' : 'Not analysed yet — using frame centre';
  } catch (err) { status(err.message, true); }
}

// ---------------------------------------------------------------- camera sampling

function markers() {
  if (state.timeline) return state.timeline.markers;
  const tl = state.project.timeline, d = duration();
  return { in: tl.in || 0, superStart: tl.superStart ?? d, animStart: tl.animStart ?? d, postStart: tl.postStart ?? d, out: tl.out ?? d };
}

function partAt(t) {
  const m = markers();
  if (t >= m.postStart) return 'post';
  if (t >= m.animStart) return 'animation';
  if (t >= m.superStart) return 'startup';
  return 'combo';
}

function camAt(t) {
  const tl = state.timeline;
  let cam;
  if (tl && tl.times.length) {
    const i = clamp(Math.floor((t - tl.times[0]) * tl.fps + 1e-6), 0, tl.times.length - 1);
    const alphas = {};
    for (const [k, v] of Object.entries(tl.alphas)) alphas[k] = v[i];
    cam = { cx: tl.cx[i], cy: tl.cy[i], zoom: tl.zoom[i], alphas };
  } else {
    const [w, h] = srcSize();
    const alphas = {};
    for (const el of state.project.overlay.elements) alphas[el.id] = el.enabled && el.parts?.[partAt(t)] !== false ? 1 : 0;
    cam = { cx: w / 2, cy: h / 2, zoom: 1, alphas };
  }
  if (state.live) Object.assign(cam, state.live);
  return cam;
}

function clampCam(cam) {
  const [w, h] = srcSize();
  const zoom = clamp(cam.zoom, 1, 6);
  const ch = h / zoom, cw = Math.min(w, ch * ASPECT);
  return { cx: clamp(cam.cx, cw / 2, w - cw / 2), cy: clamp(cam.cy, ch / 2, h - ch / 2), zoom };
}

function cropRect(cam) {
  const [, h] = srcSize();
  const ch = h / cam.zoom, cw = ch * ASPECT;
  return { x: cam.cx - cw / 2, y: cam.cy - ch / 2, w: cw, h: ch };
}

// ---------------------------------------------------------------- drawing

const offscreen = new Map();
function featherCanvas(el, dw, dh) {
  let c = offscreen.get(el.id);
  if (!c) { c = document.createElement('canvas'); offscreen.set(el.id, c); }
  if (c.width !== dw || c.height !== dh) { c.width = dw; c.height = dh; }
  return c;
}

function uiScale() {
  const ref = state.project.overlay.referenceSize || [1920, 1080];
  return [(vUi.videoWidth || ref[0]) / ref[0], (vUi.videoHeight || ref[1]) / ref[1]];
}

function drawElement(ctx, el, alpha) {
  const a = alpha * (el.opacity ?? 1);
  if (a <= 0.001 || !vUi.videoWidth) return;
  const [sx, sy] = uiScale();
  const dw = Math.max(1, Math.round(el.dst.w)), dh = Math.max(1, Math.round(el.dst.h));
  const c = featherCanvas(el, dw, dh), o = c.getContext('2d');
  o.globalCompositeOperation = 'copy';
  o.drawImage(vUi, el.src.x * sx, el.src.y * sy, el.src.w * sx, el.src.h * sy, 0, 0, dw, dh);
  const f = el.feather || {};
  const ramp = (grad, a0, a1, len) => {
    const s = Math.min(a0 / len, 0.5), e = Math.min(a1 / len, 0.5);
    grad.addColorStop(0, `rgba(0,0,0,${a0 > 0 ? 0 : 1})`);
    if (a0 > 0) grad.addColorStop(s, 'rgba(0,0,0,1)');
    if (a1 > 0) grad.addColorStop(1 - e, 'rgba(0,0,0,1)');
    grad.addColorStop(1, `rgba(0,0,0,${a1 > 0 ? 0 : 1})`);
  };
  o.globalCompositeOperation = 'destination-in';
  if (f.left > 0 || f.right > 0) {
    const g = o.createLinearGradient(0, 0, dw, 0); ramp(g, f.left || 0, f.right || 0, dw);
    o.fillStyle = g; o.fillRect(0, 0, dw, dh);
  }
  if (f.top > 0 || f.bottom > 0) {
    const g = o.createLinearGradient(0, 0, 0, dh); ramp(g, f.top || 0, f.bottom || 0, dh);
    o.fillStyle = g; o.fillRect(0, 0, dw, dh);
  }
  ctx.globalAlpha = a;
  ctx.drawImage(c, Math.round(el.dst.x), Math.round(el.dst.y));
  ctx.globalAlpha = 1;
}

function drawPreview() {
  const t = now();
  const cam = camAt(t);
  pctx.fillStyle = '#000';
  pctx.fillRect(0, 0, OUT_W, OUT_H);
  if (vClean.videoWidth) {
    const r = cropRect(cam);
    const k = vClean.videoWidth / srcSize()[0];
    pctx.imageSmoothingQuality = 'high';
    pctx.drawImage(vClean, r.x * k, r.y * k, r.w * k, r.h * k, 0, 0, OUT_W, OUT_H);
  } else {
    pctx.fillStyle = '#555'; pctx.font = '48px system-ui'; pctx.textAlign = 'center';
    pctx.fillText('Load videos to begin', OUT_W / 2, OUT_H / 2);
  }
  for (const el of state.project.overlay.elements) {
    if (el.enabled) drawElement(pctx, el, cam.alphas[el.id] ?? 0);
  }

  if (state.tab === 'overlay') {
    for (const el of state.project.overlay.elements) {
      const sel = el.id === state.selected;
      pctx.lineWidth = sel ? 4 : 2;
      pctx.setLineDash(sel ? [] : [12, 8]);
      pctx.strokeStyle = sel ? css('--accent') : 'rgba(255,255,255,.6)';
      pctx.strokeRect(el.dst.x, el.dst.y, el.dst.w, el.dst.h);
      if (sel) { pctx.fillStyle = css('--accent'); pctx.fillRect(el.dst.x + el.dst.w - 22, el.dst.y + el.dst.h - 22, 22, 22); }
    }
    pctx.setLineDash([]);
  }
  if ($('#safeZone').checked) {
    // Approximate TikTok UI: caption/buttons at bottom, action column on the right, tabs at top.
    pctx.fillStyle = 'rgba(255,40,80,.18)';
    pctx.fillRect(0, 0, OUT_W, 130);
    pctx.fillRect(0, OUT_H - 420, OUT_W, 420);
    pctx.fillRect(OUT_W - 150, 130, 150, OUT_H - 550);
  }
  const part = partAt(t);
  const super_ = part === 'startup' || part === 'animation';
  if (state.tab === 'camera' && super_) {
    pctx.strokeStyle = COLORS()[part]; pctx.lineWidth = 8; pctx.strokeRect(4, 4, OUT_W - 8, OUT_H - 8);
  }
  const pl = $('#partLabel');
  pl.textContent = PART_LABEL[part];
  pl.style.background = COLORS()[part];
  $('#camInfo').textContent = `cam x ${cam.cx.toFixed(0)} y ${cam.cy.toFixed(0)} · zoom ${cam.zoom.toFixed(2)}×`;
}

function sourceToCanvas() {
  const [w, h] = srcSize();
  return source.width / w;
}

function drawSource() {
  const t = now();
  const W = source.width, H = source.height;
  sctx.globalCompositeOperation = 'source-over';
  sctx.fillStyle = '#000';
  sctx.fillRect(0, 0, W, H);
  const mode = state.sourceMode;
  if (mode === 'ui' || mode === 'diff') {
    if (mode === 'diff' && vClean.videoWidth) sctx.drawImage(vClean, 0, 0, W, H);
    if (vUi.videoWidth) {
      sctx.globalCompositeOperation = mode === 'diff' ? 'difference' : 'source-over';
      sctx.drawImage(vUi, 0, 0, W, H);
      sctx.globalCompositeOperation = 'source-over';
    }
  } else if (vClean.videoWidth) {
    sctx.drawImage(vClean, 0, 0, W, H);
  }
  const k = sourceToCanvas();

  if (mode === 'clean') {
    // Tracking extents for this frame.
    if (state.track) {
      const i = clamp(Math.round(t * state.track.fps), 0, state.track.frames - 1);
      const ks = W / state.track.width;
      sctx.strokeStyle = 'rgba(80,220,255,.85)'; sctx.lineWidth = 1.5; sctx.setLineDash([4, 4]);
      for (const x of [state.track.left[i], state.track.right[i]]) {
        sctx.beginPath(); sctx.moveTo(x * ks, 0); sctx.lineTo(x * ks, H); sctx.stroke();
      }
      sctx.setLineDash([]);
    }
    const r = cropRect(camAt(t));
    sctx.fillStyle = 'rgba(0,0,0,.45)';
    sctx.fillRect(0, 0, r.x * k, H);
    sctx.fillRect((r.x + r.w) * k, 0, W - (r.x + r.w) * k, H);
    sctx.fillRect(r.x * k, 0, r.w * k, r.y * k);
    sctx.fillRect(r.x * k, (r.y + r.h) * k, r.w * k, H - (r.y + r.h) * k);
    sctx.strokeStyle = COLORS()[partAt(t)]; sctx.lineWidth = 2;
    sctx.strokeRect(r.x * k, r.y * k, r.w * k, r.h * k);
    sctx.beginPath(); sctx.moveTo((r.x + r.w / 2) * k, r.y * k); sctx.lineTo((r.x + r.w / 2) * k, (r.y + r.h) * k);
    sctx.globalAlpha = 0.4; sctx.stroke(); sctx.globalAlpha = 1;
  } else if (state.tab === 'overlay' || mode === 'ui') {
    const ref = state.project.overlay.referenceSize || [1920, 1080];
    const kr = W / ref[0];
    for (const el of state.project.overlay.elements) {
      const sel = el.id === state.selected;
      sctx.strokeStyle = sel ? css('--accent') : 'rgba(255,255,255,.7)';
      sctx.lineWidth = sel ? 2 : 1;
      sctx.setLineDash(sel ? [] : [5, 4]);
      sctx.strokeRect(el.src.x * kr, el.src.y * kr, el.src.w * kr, el.src.h * kr);
      if (sel) { sctx.fillStyle = css('--accent'); sctx.fillRect((el.src.x + el.src.w) * kr - 8, (el.src.y + el.src.h) * kr - 8, 8, 8); }
    }
    sctx.setLineDash([]);
  }
}

function tlRange() {
  if ($('#zoomRange').checked && state.info.clean) {
    const m = markers();
    const pad = Math.max(0.5, (m.out - m.in) * 0.05);
    return [Math.max(0, m.in - pad), Math.min(duration(), m.out + pad)];
  }
  return [0, duration() || 1];
}

function drawTimeline() {
  const dpr = window.devicePixelRatio || 1;
  const cssW = tlCanvas.clientWidth, cssH = tlCanvas.clientHeight;
  if (tlCanvas.width !== Math.round(cssW * dpr)) { tlCanvas.width = Math.round(cssW * dpr); tlCanvas.height = Math.round(cssH * dpr); }
  const W = cssW, H = cssH;
  tctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  tctx.clearRect(0, 0, W, H);
  const [a, b] = tlRange();
  const X = (t) => ((t - a) / (b - a)) * W;
  const colors = COLORS();

  // Waveform of the clean video's audio.
  if (state.waveform) {
    const { rate, peaks } = state.waveform;
    tctx.fillStyle = 'rgba(255,255,255,.13)';
    const mid = H / 2 + 6;
    for (let px = 0; px < W; px++) {
      const t0 = a + (px / W) * (b - a), t1 = a + ((px + 1) / W) * (b - a);
      let p = 0;
      for (let i = Math.floor(t0 * rate); i <= Math.floor(t1 * rate) && i < peaks.length; i++) p = Math.max(p, peaks[i] || 0);
      const hgt = Math.min(1, p) * (H - 30) / 2;
      tctx.fillRect(px, mid - hgt, 1, hgt * 2);
    }
  }
  if (!state.info.clean) return;
  const m = markers();
  const bands = [['combo', m.in, m.superStart], ['startup', m.superStart, m.animStart], ['animation', m.animStart, m.postStart], ['post', m.postStart, m.out]];
  for (const [p, s, e] of bands) {
    if (e <= s) continue;
    tctx.fillStyle = colors[p];
    tctx.globalAlpha = 0.22; tctx.fillRect(X(s), 16, X(e) - X(s), H - 16);
    tctx.globalAlpha = 1; tctx.fillRect(X(s), 16, X(e) - X(s), 4);
  }
  tctx.fillStyle = 'rgba(0,0,0,.45)';
  tctx.fillRect(0, 16, X(m.in), H); tctx.fillRect(X(m.out), 16, W - X(m.out), H);

  // Time ruler.
  tctx.fillStyle = css('--muted'); tctx.font = '10px system-ui'; tctx.textAlign = 'left';
  const span = b - a, step = [0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60].find((s) => span / s < W / 70) || 120;
  for (let t = Math.ceil(a / step) * step; t <= b; t += step) {
    tctx.fillRect(X(t), 0, 1, 5);
    tctx.fillText(fmtTime(t).replace(/\.?0+$/, ''), X(t) + 3, 11);
  }

  // Markers.
  const set = state.project.timeline;
  for (const [key, label] of MARKERS) {
    if (set[key] == null && key !== 'in') continue;
    const x = X(m[key]);
    tctx.fillStyle = key === 'in' || key === 'out' ? '#fff' : colors[{ superStart: 'startup', animStart: 'animation', postStart: 'post' }[key]];
    tctx.fillRect(x - 1, 16, 2, H - 16);
    tctx.fillText(label, x + 4, H - 4);
  }
  // Keyframes.
  for (const k of state.project.camera.super.keyframes) {
    const x = X(k.t), y = 32;
    tctx.fillStyle = '#fff';
    tctx.beginPath(); tctx.moveTo(x, y - 6); tctx.lineTo(x + 6, y); tctx.lineTo(x, y + 6); tctx.lineTo(x - 6, y); tctx.fill();
  }
  // Playhead.
  const x = X(now());
  tctx.fillStyle = css('--accent');
  tctx.fillRect(x - 1, 0, 2, H);
  tctx.beginPath(); tctx.moveTo(x - 6, 0); tctx.lineTo(x + 6, 0); tctx.lineTo(x, 8); tctx.fill();
}

function drawEnvelopes() {
  const c = $('#envelopes'), ctx = c.getContext('2d');
  const W = c.width, H = c.height;
  ctx.clearRect(0, 0, W, H);
  ctx.fillStyle = css('--muted'); ctx.font = '11px system-ui';
  if (!state.envelopes) { ctx.fillText('Run auto-sync to compare transients', 10, H / 2); return; }
  const { hop, ui, clean } = state.envelopes;
  const off = Number(state.project.sync.offset) || 0;
  const span = 6; // seconds shown, starting at the first loud clean transient
  const first = Math.max(0, clean.findIndex((v) => v > 0.3) * hop - 0.5);
  const lane = (vals, shift, y, color) => {
    ctx.strokeStyle = color; ctx.beginPath();
    for (let px = 0; px < W; px++) {
      const t = first + (px / W) * span + shift;
      const v = vals[Math.round(t / hop)] || 0;
      const yy = y - v * (H / 2 - 12);
      px ? ctx.lineTo(px, yy) : ctx.moveTo(px, yy);
    }
    ctx.stroke();
  };
  lane(clean, 0, H / 2 - 2, css('--post'));
  lane(ui, off, H - 4, css('--accent'));
  ctx.fillText('clean', 6, 12); ctx.fillText('UI (with offset)', 6, H / 2 + 12);
}

function frameLoop() {
  syncMedia();
  drawPreview();
  drawSource();
  drawTimeline();
  $('#timecode').textContent = `${fmtTime(now())}  f${frameOf(now())}`;
  requestAnimationFrame(frameLoop);
}

// ---------------------------------------------------------------- playback

function syncMedia() {
  const t = now();
  const off = Number(state.project.sync.offset) || 0;
  if (vUi.src) {
    const target = t + off;
    const drift = Math.abs(vUi.currentTime - target);
    if (vClean.paused) {
      if (drift > 0.5 / fps() && !vUi.seeking && target >= 0) vUi.currentTime = target;
    } else if (drift > 0.06) {
      vUi.currentTime = Math.max(0, target);
    }
    if (vClean.paused !== vUi.paused) vClean.paused ? vUi.pause() : vUi.play().catch(() => {});
  }
  const m = markers();
  if (!vClean.paused && t >= m.out - 0.5 / fps()) {
    if ($('#loop').checked) seek(m.in); else vClean.pause();
  }
  // Game audio from the primary source.
  const primaryUi = state.project.audio.primary === 'ui';
  const gv = clamp(Number(state.project.audio.gameVolume) || 0, 0, 1);
  vClean.muted = primaryUi; vUi.muted = !primaryUi;
  vClean.volume = gv; vUi.volume = gv;
  // Music.
  const mu = state.project.music;
  if (aMusic.src && $('#musicPreview').checked && !vClean.paused && t >= m.in && t < m.out) {
    const local = t - m.in, len = m.out - m.in;
    let g = Number(mu.volume) || 0;
    if (mu.fadeIn > 0) g *= clamp(local / mu.fadeIn, 0, 1);
    if (mu.fadeOut > 0) g *= clamp((len - local) / mu.fadeOut, 0, 1);
    aMusic.volume = clamp(g, 0, 1);
    const target = (Number(mu.start) || 0) + local;
    if (Math.abs(aMusic.currentTime - target) > 0.15) aMusic.currentTime = target;
    if (aMusic.paused) aMusic.play().catch(() => {});
  } else if (!aMusic.paused) {
    aMusic.pause();
  }
}

function seek(t) {
  t = clamp(t, 0, duration());
  vClean.currentTime = t;
  if (vUi.src) vUi.currentTime = Math.max(0, t + (Number(state.project.sync.offset) || 0));
  renderKeyTable();
}

function togglePlay() {
  if (!vClean.src) return;
  if (vClean.paused) {
    const m = markers();
    if (now() < m.in || now() >= m.out - 1 / fps()) seek(m.in);
    vClean.play();
    $('#play').textContent = '⏸';
  } else {
    vClean.pause();
    seek(snap(now()));
    $('#play').textContent = '▶';
  }
}
vClean.addEventListener('pause', () => ($('#play').textContent = '▶'));

// ---------------------------------------------------------------- sources & sync

async function loadSources() {
  const ui = $('#uiPath').value.trim(), clean = $('#cleanPath').value.trim();
  state.project.sources = { ui, clean };
  if (!clean) return status('Choose at least the clean (no-UI) video', true);
  await guarded(async () => {
    state.info.clean = await api('/api/probe', { path: clean });
    $('#cleanInfo').textContent = `${state.info.clean.width}×${state.info.clean.height} · ${state.info.clean.fps.toFixed(2)} fps · ${state.info.clean.duration.toFixed(2)} s`;
    vClean.src = `/api/media?path=${encodeURIComponent(clean)}`;
    if (ui) {
      state.info.ui = await api('/api/probe', { path: ui });
      $('#uiInfo').textContent = `${state.info.ui.width}×${state.info.ui.height} · ${state.info.ui.fps.toFixed(2)} fps · ${state.info.ui.duration.toFixed(2)} s`;
      vUi.src = `/api/media?path=${encodeURIComponent(ui)}`;
      if (Math.abs(state.info.ui.fps - state.info.clean.fps) > 0.01) status('Warning: the two videos have different frame rates', true);
    } else {
      vUi.removeAttribute('src'); vUi.load();
    }
    if (state.project.timeline.out == null) state.project.timeline.out = state.info.clean.duration;
    changed();
    await refreshTimeline();
    renderAll();
    status('Videos loaded');
    api('/api/waveform', { path: clean }).then((w) => (state.waveform = w)).catch(() => {});
    if (state.timeline?.tracked) loadTrack();
  }, 'Loading videos…');
}

async function autoSync() {
  const { ui, clean } = state.project.sources;
  if (!ui || !clean) return status('Load both videos first', true);
  await guarded(async () => {
    const r = await api('/api/sync', { ui, clean });
    state.project.sync.offset = Number(r.offset.toFixed(4));
    state.project.sync.confidence = r.confidence;
    state.envelopes = r.envelopes;
    renderSync();
    seek(now());
    changed({ timeline: false });
    status(`Synced: offset ${r.offset.toFixed(3)} s`);
  }, 'Analysing transients…');
}

function renderSync() {
  $('#offset').value = state.project.sync.offset;
  const c = state.project.sync.confidence;
  $('#syncConf').textContent = c == null ? '' : `confidence ${(c * 100).toFixed(0)}%` + (c < 0.25 ? ' — check by eye' : '');
  const f = Number(state.project.sync.offset) * fps();
  $('#syncConf').title = `${f.toFixed(2)} frames`;
  drawEnvelopes();
}

async function upload(input) {
  const file = input.files[0];
  if (!file) return;
  const fd = new FormData();
  fd.append('file', file);
  await guarded(async () => {
    const res = await fetch('/api/upload', { method: 'POST', body: fd });
    if (!res.ok) throw new Error('Upload failed');
    const { path } = await res.json();
    const target = $('#' + input.dataset.upload);
    target.value = path;
    target.dispatchEvent(new Event('change'));
    status('Uploaded ' + file.name);
  }, `Uploading ${file.name}…`);
}

// ---------------------------------------------------------------- tracking

async function runTrack() {
  await guarded(async () => {
    $('#trackStatus').textContent = 'Analysing…';
    await loadTrack();
    await refreshTimeline();
    status('Fighters tracked');
  }, 'Tracking fighters (this can take a moment)…');
}

async function loadTrack() {
  state.track = await api('/api/track', state.project);
}

// ---------------------------------------------------------------- markers & keyframes

function setMarker(key, t) {
  const tl = state.project.timeline;
  tl[key] = t == null ? null : Number(snap(t).toFixed(4));
  // Keep later markers after earlier ones.
  const order = MARKERS.map(([k]) => k);
  const idx = order.indexOf(key);
  for (const k of order.slice(idx + 1)) if (tl[k] != null && tl[key] != null && tl[k] < tl[key]) tl[k] = tl[key];
  for (const k of order.slice(0, idx)) if (tl[k] != null && tl[key] != null && tl[k] > tl[key]) tl[k] = tl[key];
  renderMarkers();
  changed();
}

function renderMarkers() {
  const table = $('#markerTable');
  table.innerHTML = '';
  for (const [key, label] of MARKERS) {
    const tr = document.createElement('tr');
    const v = state.project.timeline[key];
    tr.innerHTML = `<td>${label}</td><td><input type="number" step="${(1 / fps()).toFixed(4)}" value="${v ?? ''}" placeholder="unset"></td>
      <td><button data-act="set">Set</button></td><td><button data-act="go">Go</button></td><td><button data-act="clear">✕</button></td>`;
    $('input', tr).addEventListener('change', (e) => setMarker(key, e.target.value === '' ? null : Number(e.target.value)));
    $('[data-act=set]', tr).onclick = () => setMarker(key, now());
    $('[data-act=go]', tr).onclick = () => v != null && seek(v);
    $('[data-act=clear]', tr).onclick = () => setMarker(key, key === 'in' ? 0 : null);
    table.append(tr);
  }
}

function keyframeAt(t, create) {
  const keys = state.project.camera.super.keyframes;
  let k = keys.find((k) => Math.abs(k.t - t) < 0.5 / fps());
  if (!k && create) {
    const cam = camAt(t);
    k = { t: Number(snap(t).toFixed(4)), cx: Number(cam.cx.toFixed(1)), cy: Number(cam.cy.toFixed(1)), zoom: Number(cam.zoom.toFixed(3)) };
    keys.push(k);
    keys.sort((a, b) => a.t - b.t);
  }
  return k;
}

function inSuper(t) {
  const p = partAt(t);
  return p === 'startup' || p === 'animation';
}

function editKeyframe(fn) {
  const t = now();
  if (!inSuper(t)) { status('Move the playhead into the Super Art section (set its markers first)', true); return false; }
  const k = keyframeAt(t, true);
  const next = clampCam(fn({ cx: k.cx, cy: k.cy, zoom: k.zoom }));
  k.cx = Number(next.cx.toFixed(1)); k.cy = Number(next.cy.toFixed(1)); k.zoom = Number(next.zoom.toFixed(3));
  state.live = { cx: k.cx, cy: k.cy, zoom: k.zoom };
  renderKeyTable();
  changed();
  return true;
}

function renderKeyTable() {
  const table = $('#keyTable');
  if (!table) return;
  const keys = state.project.camera.super.keyframes;
  table.innerHTML = keys.length ? '<tr><td>time</td><td>x</td><td>y</td><td>zoom</td><td></td><td></td></tr>' : '<tr><td class="muted">No keyframes — the camera holds where the combo ended.</td></tr>';
  keys.forEach((k, i) => {
    const tr = document.createElement('tr');
    if (Math.abs(k.t - now()) < 0.5 / fps()) tr.className = 'current';
    tr.innerHTML = `<td>${fmtTime(k.t)}</td>
      <td><input type="number" step="1" data-k="cx" value="${k.cx}"></td>
      <td><input type="number" step="1" data-k="cy" value="${k.cy}"></td>
      <td><input type="number" step="0.01" min="1" data-k="zoom" value="${k.zoom}"></td>
      <td><button data-act="go">Go</button></td><td><button data-act="del">✕</button></td>`;
    $$('input', tr).forEach((inp) => inp.addEventListener('change', () => { k[inp.dataset.k] = Number(inp.value); Object.assign(k, clampCam(k)); changed(); renderKeyTable(); }));
    $('[data-act=go]', tr).onclick = () => seek(k.t);
    $('[data-act=del]', tr).onclick = () => { keys.splice(i, 1); renderKeyTable(); changed(); };
    table.append(tr);
  });
}

// ---------------------------------------------------------------- overlay elements

const selectedEl = () => state.project.overlay.elements.find((e) => e.id === state.selected);

function renderElements() {
  const list = $('#elementList');
  list.innerHTML = '';
  for (const el of state.project.overlay.elements) {
    const b = document.createElement('button');
    b.className = el.id === state.selected ? 'active' : '';
    b.innerHTML = `<span>${el.label || el.id}</span><span class="muted">${el.enabled ? '' : 'off'}</span>`;
    b.onclick = () => { state.selected = el.id; renderElements(); };
    list.append(b);
  }
  renderElementEditor();
}

function renderElementEditor() {
  const el = selectedEl();
  $('#elEditor').classList.toggle('hidden', !el);
  if (!el) return;
  $('#elLabel').value = el.label || '';
  $('#elEnabled').checked = !!el.enabled;
  $$('[data-rect]').forEach((inp) => (inp.value = Math.round(getPath(el, inp.dataset.rect))));
  $$('[data-blend]').forEach((inp) => (inp.value = getPath(el, inp.dataset.blend) ?? 0));
  $('#elOpacityOut').textContent = Number(el.opacity ?? 1).toFixed(2);
  $$('[data-bout]').forEach((o) => (o.textContent = el.feather?.[o.dataset.bout] ?? 0));
  $$('[data-part]').forEach((c) => (c.checked = el.parts?.[c.dataset.part] !== false));
}

function newElementId() {
  let i = 1;
  while (state.project.overlay.elements.some((e) => e.id === `el${i}`)) i++;
  return `el${i}`;
}

// ---------------------------------------------------------------- presets & projects

async function refreshPresetLists() {
  await guarded(async () => {
    const [skins, blends, projects] = await Promise.all([api('/api/presets/skins'), api('/api/presets/blends'), api('/api/presets/projects')]);
    const fill = (sel, names, current) => {
      sel.innerHTML = names.map((n) => `<option>${n.replace(/</g, '&lt;')}</option>`).join('');
      if (current && names.includes(current)) sel.value = current;
    };
    fill($('#skinList'), skins, state.project.overlay.name);
    fill($('#blendList'), blends);
    fill($('#projectList'), projects, state.project.name);
  });
}

const enc = encodeURIComponent;

async function saveSkin(name) {
  const skin = structuredClone(state.project.overlay);
  skin.name = name;
  await api(`/api/presets/skins/${enc(name)}`, skin, 'PUT');
  state.project.overlay.name = name;
  await refreshPresetLists();
  status(`Saved skin “${name}”`);
}

// ---------------------------------------------------------------- generic bindings

function bindPaths() {
  for (const inp of $$('[data-path]')) {
    const path = inp.dataset.path;
    const v = getPath(state.project, path);
    if (inp.type === 'checkbox') inp.checked = !!v; else inp.value = v ?? '';
    const out = inp.id && $(`output[data-for="${inp.id}"]`);
    if (out) out.textContent = inp.value;
  }
  $$('input[name=primary]').forEach((r) => (r.checked = r.value === state.project.audio.primary));
}

function renderAll() {
  $('#projectName').value = state.project.name;
  $('#uiPath').value = state.project.sources.ui;
  $('#cleanPath').value = state.project.sources.clean;
  bindPaths();
  renderSync();
  renderMarkers();
  renderKeyTable();
  if (!selectedEl()) state.selected = state.project.overlay.elements[0]?.id ?? null;
  renderElements();
  aMusic.src = state.project.music.path ? `/api/media?path=${enc(state.project.music.path)}` : '';
}

const TIMELINE_PATHS = /^(camera|tracking|overlay\.fade|timeline)/;

function setupEvents() {
  $$('.tabs button').forEach((b) => (b.onclick = () => {
    state.tab = b.dataset.tab;
    $$('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    $$('.tab').forEach((s) => s.classList.toggle('hidden', s.id !== `tab-${state.tab}`));
    if (state.tab === 'overlay') setSourceMode('ui');
    else if (state.tab === 'camera') setSourceMode('clean');
  }));
  $$('#sourceMode button').forEach((b) => (b.onclick = () => setSourceMode(b.dataset.mode)));

  for (const inp of $$('[data-path]')) {
    const handler = () => {
      let v = inp.type === 'checkbox' ? inp.checked : inp.value;
      if (inp.type === 'number' || inp.type === 'range') v = v === '' ? 0 : Number(v);
      setPath(state.project, inp.dataset.path, v);
      const out = inp.id && $(`output[data-for="${inp.id}"]`);
      if (out) out.textContent = inp.value;
      if (inp.dataset.path === 'music.path') aMusic.src = v ? `/api/media?path=${enc(v)}` : '';
      changed({ timeline: TIMELINE_PATHS.test(inp.dataset.path) });
    };
    inp.addEventListener(inp.type === 'range' ? 'input' : 'change', handler);
  }
  $$('input[type=file][data-upload]').forEach((i) => i.addEventListener('change', () => upload(i)));
  $$('input[name=primary]').forEach((r) => r.addEventListener('change', () => { state.project.audio.primary = r.value; changed({ timeline: false }); }));

  $('#loadSources').onclick = loadSources;
  $('#autoSync').onclick = autoSync;
  $('#offset').addEventListener('change', (e) => { state.project.sync.offset = Number(e.target.value); renderSync(); seek(now()); changed({ timeline: false }); });
  $$('[data-nudge]').forEach((b) => (b.onclick = () => {
    const n = b.dataset.nudge;
    const d = n.endsWith('f') ? Number(n.slice(0, -1)) / fps() : Number(n);
    state.project.sync.offset = Number((Number(state.project.sync.offset) + d).toFixed(4));
    renderSync(); seek(now()); changed({ timeline: false });
  }));
  $('#snapOffset').onclick = () => {
    state.project.sync.offset = Number((Math.round(state.project.sync.offset * fps()) / fps()).toFixed(5));
    renderSync(); seek(now()); changed({ timeline: false });
  };

  $('#runTrack').onclick = runTrack;
  $('#addKey').onclick = () => editKeyframe((c) => c);

  // Transport.
  $('#play').onclick = togglePlay;
  $('#stepBack').onclick = () => { vClean.pause(); seek(snap(now()) - 1 / fps()); };
  $('#stepFwd').onclick = () => { vClean.pause(); seek(snap(now()) + 1 / fps()); };
  $$('[data-mark]').forEach((b) => (b.onclick = () => setMarker(b.dataset.mark, now())));
  document.addEventListener('keydown', (e) => {
    if (e.target.closest('input, select, textarea')) return;
    const f = 1 / fps();
    if (e.code === 'Space') { e.preventDefault(); togglePlay(); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); vClean.pause(); seek(snap(now()) - (e.shiftKey ? 1 : f)); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); vClean.pause(); seek(snap(now()) + (e.shiftKey ? 1 : f)); }
    else if (e.key === 'i' || e.key === 'I') setMarker('in', now());
    else if (e.key === 'o' || e.key === 'O') setMarker('out', now());
    else if (e.key === 'k' || e.key === 'K') editKeyframe((c) => c);
  });

  setupTimelineMouse();
  setupPreviewMouse();
  setupSourceMouse();

  // Overlay editor.
  $('#elLabel').addEventListener('change', (e) => { selectedEl().label = e.target.value; renderElements(); changed({ timeline: false }); });
  $('#elEnabled').addEventListener('change', (e) => { selectedEl().enabled = e.target.checked; renderElements(); changed(); });
  $$('[data-rect]').forEach((inp) => inp.addEventListener('change', () => {
    setPath(selectedEl(), inp.dataset.rect, Math.max(inp.dataset.rect.endsWith('w') || inp.dataset.rect.endsWith('h') ? 1 : -9999, Number(inp.value)));
    changed({ timeline: false });
  }));
  $$('[data-blend]').forEach((inp) => inp.addEventListener('input', () => {
    const el = selectedEl();
    el.feather ??= {};
    setPath(el, inp.dataset.blend, Number(inp.value));
    renderElementEditor();
    changed({ timeline: false });
  }));
  $$('[data-part]').forEach((c) => c.addEventListener('change', () => {
    const el = selectedEl();
    el.parts ??= {};
    el.parts[c.dataset.part] = c.checked;
    changed();
  }));
  $('#elMatchAspect').onclick = () => {
    const el = selectedEl();
    el.dst.h = Math.round(el.dst.w * el.src.h / el.src.w);
    renderElementEditor(); changed({ timeline: false });
  };
  $('#elCenter').onclick = () => { const el = selectedEl(); el.dst.x = Math.round((OUT_W - el.dst.w) / 2); renderElementEditor(); changed({ timeline: false }); };
  $('#elAdd').onclick = () => {
    const id = newElementId();
    state.project.overlay.elements.push({
      id, label: 'New element', enabled: true, src: { x: 760, y: 400, w: 400, h: 120 }, dst: { x: 240, y: 500, w: 600, h: 180 },
      opacity: 1, feather: { left: 0, right: 0, top: 0, bottom: 0 }, parts: { combo: true, startup: true, animation: false, post: true },
    });
    state.selected = id; renderElements(); changed();
  };
  $('#elDup').onclick = () => {
    const el = selectedEl(); if (!el) return;
    const copy = structuredClone(el); copy.id = newElementId(); copy.label = `${el.label} copy`; copy.dst.y += 40;
    state.project.overlay.elements.push(copy); state.selected = copy.id; renderElements(); changed();
  };
  $('#elDelete').onclick = () => {
    const els = state.project.overlay.elements, i = els.findIndex((e) => e.id === state.selected);
    if (i < 0 || !confirm(`Delete “${els[i].label}”?`)) return;
    els.splice(i, 1); state.selected = els[0]?.id ?? null; renderElements(); changed();
  };

  // Presets.
  $('#skinLoad').onclick = () => guarded(async () => {
    const skin = await api(`/api/presets/skins/${enc($('#skinList').value)}`);
    state.project.overlay = skin;
    state.selected = skin.elements[0]?.id ?? null;
    renderAll(); changed(); status(`Loaded skin “${skin.name}”`);
  });
  $('#skinSave').onclick = () => guarded(() => saveSkin(state.project.overlay.name || 'Default'));
  $('#skinSaveAs').onclick = () => { const n = prompt('Skin preset name', state.project.overlay.name || ''); if (n) guarded(() => saveSkin(n)); };
  $('#skinDelete').onclick = () => {
    const n = $('#skinList').value;
    if (n && confirm(`Delete skin preset “${n}”?`)) guarded(async () => { await api(`/api/presets/skins/${enc(n)}`, undefined, 'DELETE'); await refreshPresetLists(); });
  };
  $('#blendApply').onclick = () => guarded(async () => {
    const el = selectedEl(); if (!el) return;
    const b = await api(`/api/presets/blends/${enc($('#blendList').value)}`);
    el.opacity = b.opacity; el.feather = structuredClone(b.feather);
    renderElementEditor(); changed({ timeline: false }); status(`Applied blend “${b.name}”`);
  });
  $('#blendSaveAs').onclick = () => {
    const el = selectedEl(); if (!el) return;
    const n = prompt('Blend preset name'); if (!n) return;
    guarded(async () => {
      await api(`/api/presets/blends/${enc(n)}`, { name: n, opacity: el.opacity, feather: el.feather }, 'PUT');
      await refreshPresetLists(); $('#blendList').value = n; status(`Saved blend “${n}”`);
    });
  };
  $('#blendDelete').onclick = () => {
    const n = $('#blendList').value;
    if (n && confirm(`Delete blend preset “${n}”?`)) guarded(async () => { await api(`/api/presets/blends/${enc(n)}`, undefined, 'DELETE'); await refreshPresetLists(); });
  };

  // Projects.
  $('#projectName').addEventListener('change', (e) => { state.project.name = e.target.value.trim() || 'untitled'; save(); });
  $('#projectSave').onclick = () => guarded(async () => {
    state.project.name = $('#projectName').value.trim() || 'untitled';
    await api(`/api/presets/projects/${enc(state.project.name)}`, state.project, 'PUT');
    await refreshPresetLists(); status(`Saved project “${state.project.name}”`);
  });
  $('#projectLoad').onclick = () => guarded(async () => {
    const n = $('#projectList').value; if (!n) return;
    const defaults = await api('/api/defaults');
    state.project = deepMerge(defaults, await api(`/api/presets/projects/${enc(n)}`));
    renderAll();
    await loadSources();
  });
  $('#projectNew').onclick = () => guarded(async () => {
    if (!confirm('Start a new project? Unsaved changes are lost.')) return;
    state.project = await api('/api/defaults');
    state.timeline = state.track = state.waveform = state.envelopes = null;
    state.info = { clean: null, ui: null };
    vClean.removeAttribute('src'); vUi.removeAttribute('src'); vClean.load(); vUi.load();
    renderAll(); save();
  });

  // Export.
  $('#exportBtn').onclick = startExport;
  $('#stillBtn').onclick = () => guarded(async () => {
    const blob = await api('/api/still', { project: state.project, t: now() });
    $('#stillImg').src = URL.createObjectURL(blob);
    $('#stillWrap').classList.remove('hidden');
    status('');
  }, 'Rendering exact frame…');
  $('#stillClose').onclick = () => $('#stillWrap').classList.add('hidden');
}

function setSourceMode(mode) {
  state.sourceMode = mode;
  $$('#sourceMode button').forEach((b) => b.classList.toggle('active', b.dataset.mode === mode));
}

function deepMerge(base, over) {
  const out = structuredClone(base);
  for (const [k, v] of Object.entries(over || {})) {
    out[k] = v && typeof v === 'object' && !Array.isArray(v) && out[k] && typeof out[k] === 'object' ? deepMerge(out[k], v) : structuredClone(v);
  }
  return out;
}

// ---------------------------------------------------------------- mouse interaction

function canvasPoint(canvas, e) {
  const r = canvas.getBoundingClientRect();
  return [((e.clientX - r.left) / r.width) * canvas.width, ((e.clientY - r.top) / r.height) * canvas.height];
}

function dragLoop(onMove, onUp) {
  const move = (e) => onMove(e);
  const up = (e) => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); onUp?.(e); };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

function rectDrag(canvas, rect, scale, handle, e, lockAspect, onChange) {
  const [x0, y0] = canvasPoint(canvas, e);
  const start = { ...rect };
  const aspect = start.w / start.h;
  dragLoop((ev) => {
    const [x, y] = canvasPoint(canvas, ev);
    const dx = (x - x0) / scale, dy = (y - y0) / scale;
    if (handle) {
      rect.w = Math.max(4, Math.round(start.w + dx));
      rect.h = lockAspect && !ev.altKey ? Math.max(2, Math.round(rect.w / aspect)) : Math.max(2, Math.round(start.h + dy));
    } else {
      rect.x = Math.round(start.x + dx);
      rect.y = Math.round(start.y + dy);
    }
    renderElementEditor();
  }, () => onChange());
}

function hitRect(rect, x, y, scale, handleSize) {
  const rx = rect.x * scale, ry = rect.y * scale, rw = rect.w * scale, rh = rect.h * scale;
  if (x >= rx + rw - handleSize && x <= rx + rw + 4 && y >= ry + rh - handleSize && y <= ry + rh + 4) return 'handle';
  if (x >= rx && x <= rx + rw && y >= ry && y <= ry + rh) return 'body';
  return null;
}

function pickElement(x, y, key, scale, handleSize) {
  const els = state.project.overlay.elements;
  const ordered = [...els.filter((e) => e.id === state.selected), ...els.filter((e) => e.id !== state.selected)];
  for (const el of ordered) {
    const hit = hitRect(el[key], x, y, scale, handleSize);
    if (hit) return [el, hit];
  }
  return [null, null];
}

function setupPreviewMouse() {
  preview.addEventListener('pointerdown', (e) => {
    const [x, y] = canvasPoint(preview, e);
    if (state.tab === 'overlay') {
      const [el, hit] = pickElement(x, y, 'dst', 1, 40);
      if (!el) return;
      state.selected = el.id; renderElements();
      rectDrag(preview, el.dst, 1, hit === 'handle', e, true, () => changed({ timeline: false }));
      return;
    }
    if (state.tab === 'camera' && inSuper(now())) {
      vClean.pause();
      let [lx, ly] = [x, y];
      dragLoop((ev) => {
        const [mx, my] = canvasPoint(preview, ev);
        const k = (srcSize()[1] / camAt(now()).zoom) / OUT_H; // source px per output px
        editKeyframe((c) => ({ ...c, cx: c.cx - (mx - lx) * k, cy: c.cy - (my - ly) * k }));
        [lx, ly] = [mx, my];
      });
    }
  });
  preview.addEventListener('wheel', (e) => {
    if (state.tab !== 'camera' || !inSuper(now())) return;
    e.preventDefault();
    vClean.pause();
    editKeyframe((c) => ({ ...c, zoom: c.zoom * Math.pow(1.0015, -e.deltaY) }));
  }, { passive: false });
}

function setupSourceMouse() {
  source.addEventListener('pointerdown', (e) => {
    const [x, y] = canvasPoint(source, e);
    if (state.sourceMode === 'ui' || state.tab === 'overlay') {
      const ref = state.project.overlay.referenceSize || [1920, 1080];
      const k = source.width / ref[0];
      const [el, hit] = pickElement(x, y, 'src', k, 14);
      if (!el) return;
      state.selected = el.id; renderElements();
      rectDrag(source, el.src, k, hit === 'handle', e, false, () => changed({ timeline: false }));
      return;
    }
    if (state.tab === 'camera' && inSuper(now())) {
      vClean.pause();
      const k = sourceToCanvas();
      let [lx, ly] = [x, y];
      dragLoop((ev) => {
        const [mx, my] = canvasPoint(source, ev);
        editKeyframe((c) => ({ ...c, cx: c.cx + (mx - lx) / k, cy: c.cy + (my - ly) / k }));
        [lx, ly] = [mx, my];
      });
    }
  });
  source.addEventListener('wheel', (e) => {
    if (state.tab !== 'camera' || !inSuper(now())) return;
    e.preventDefault();
    editKeyframe((c) => ({ ...c, zoom: c.zoom * Math.pow(1.0015, -e.deltaY) }));
  }, { passive: false });
}

function setupTimelineMouse() {
  const timeAt = (e) => {
    const r = tlCanvas.getBoundingClientRect();
    const [a, b] = tlRange();
    return a + ((e.clientX - r.left) / r.width) * (b - a);
  };
  tlCanvas.addEventListener('pointerdown', (e) => {
    if (!state.info.clean) return;
    const r = tlCanvas.getBoundingClientRect();
    const [a, b] = tlRange();
    const px = (t) => ((t - a) / (b - a)) * r.width;
    const x = e.clientX - r.left, y = e.clientY - r.top;
    const m = markers();
    vClean.pause();
    // Keyframe diamonds (top lane) can be dragged in time.
    const keys = state.project.camera.super.keyframes;
    const key = y < 44 && y > 22 && keys.find((k) => Math.abs(px(k.t) - x) < 7);
    if (key) {
      dragLoop((ev) => {
        const s = markers();
        key.t = Number(snap(clamp(timeAt(ev), s.superStart + 1 / fps(), s.postStart)).toFixed(4));
        keys.sort((p, q) => p.t - q.t);
        seek(key.t); changed();
      });
      return;
    }
    const marker = y > 16 && MARKERS.map(([k]) => k).find((k) => (state.project.timeline[k] != null || k === 'in') && Math.abs(px(m[k]) - x) < 5);
    if (marker) {
      dragLoop((ev) => { setMarker(marker, clamp(timeAt(ev), 0, duration())); seek(state.project.timeline[marker]); });
      return;
    }
    seek(snap(timeAt(e)));
    dragLoop((ev) => seek(snap(timeAt(ev))));
  });
}

// ---------------------------------------------------------------- export

let exportJob = null;
async function startExport() {
  if (!state.project.sources.clean) return status('Load videos first', true);
  await guarded(async () => {
    const { id, output } = await api('/api/export', state.project);
    exportJob = id;
    $('#exportBtn').disabled = true;
    $('#exportCancel').classList.remove('hidden');
    $('#exportProgress').classList.remove('hidden');
    $('#exportMsg').textContent = `Exporting to ${output}`;
    $('#exportCancel').onclick = () => api(`/api/jobs/${id}/cancel`, {});
    const poll = async () => {
      const job = await api(`/api/jobs/${id}`);
      $('#exportProgress').value = job.progress;
      $('#exportMsg').textContent = job.message;
      if (job.status === 'running') return setTimeout(poll, 500);
      $('#exportBtn').disabled = false;
      $('#exportCancel').classList.add('hidden');
      exportJob = null;
      if (job.status === 'done') {
        $('#exportMsg').innerHTML = `Done → <a href="/api/media?path=${enc(job.output)}" target="_blank" rel="noopener">${job.output.replace(/</g, '&lt;')}</a>`;
        const reveal = $('#revealOut');
        reveal.classList.remove('hidden');
        reveal.onclick = () => window.pywebview?.api.reveal(job.output);
        status('Export finished');
      } else {
        status(job.message, job.status === 'error');
      }
    };
    poll();
  }, 'Starting export…');
}

// ---------------------------------------------------------------- desktop (pywebview) integration

function setupDesktop() {
  // In the Windows app the page runs inside a native window that exposes file dialogs.
  document.body.classList.add('desktop');
  $$('[data-browse]').forEach((b) => (b.onclick = async () => {
    const path = await window.pywebview.api.pick_file(b.dataset.kind);
    if (!path) return;
    const target = $('#' + b.dataset.browse);
    target.value = path;
    target.dispatchEvent(new Event('change'));
    if (b.dataset.browse !== 'musicPath' && $('#uiPath').value && $('#cleanPath').value) loadSources();
  }));
  $('#outBrowse').onclick = async () => {
    const path = await window.pywebview.api.pick_save(`${state.project.name || 'export'}.mp4`);
    if (!path) return;
    $('#outPath').value = path;
    $('#outPath').dispatchEvent(new Event('change'));
  };
}

// ---------------------------------------------------------------- boot

async function boot() {
  const defaults = await api('/api/defaults');
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem('comboEditor.project') || 'null'); } catch { /* ignore */ }
  state.project = deepMerge(defaults, saved || {});
  setupEvents();
  if (window.pywebview?.api) setupDesktop();
  else window.addEventListener('pywebviewready', setupDesktop, { once: true });
  renderAll();
  await refreshPresetLists();
  requestAnimationFrame(frameLoop);
  if (state.project.sources.clean) loadSources();
}

boot().catch((err) => status(err.message, true));
