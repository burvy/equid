// Equid web app: one cost field, three views (map, warp, 3D).
// Data flow: controls → metric JSON → engine.field (WASM) → node/place costs → layers.
// Only weights, mode, time and origin call the engine; view, budget and α are pure display math.
import 'maplibre-gl/dist/maplibre-gl.css';
import './style.css';
import * as maplibregl from 'maplibre-gl';
// bundle MapLibre's tile worker with Vite (its default lookup next to the library file breaks once bundled)
import mapWorkerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
import { MapboxOverlay } from '@deck.gl/mapbox';
import { PathLayer, ScatterplotLayer, TextLayer } from '@deck.gl/layers';
import { COORDINATE_SYSTEM, type PickingInfo } from '@deck.gl/core';
import init, { Engine, Field } from './pkg/engine.js';

// ---------- types ----------
type GNode = { lon: number; lat: number; elev_m: number };
type GEdge = { from: number; to: number; highway: string; steps?: boolean };
type Graph = { meta: { bbox: [number, number, number, number]; region: string; sunrise_min: number; sunset_min: number }; nodes: GNode[]; edges: GEdge[] };
/** How far we are between views: 0..1 warp blend, lift = metres of height per second of cost. */
type ViewBlend = { warp: number; lift: number; metresPerSecond: number };
type Place = { id: string; name: string | null; category: string; lon: number; lat: number; node: number; rating: number | null; n: number | null };
type MetricSpec = { mode: 'foot' | 'bike'; weights: Record<string, number>; forbid: string[] };
type Preset = MetricSpec & { id: string; name: string; description: string };
type Feature = { id: string; unit: string; desc: string };
type View = 'map' | 'warp' | '3d';

// ---------- datasets: every data/<region>/ with graph.json + places.json ----------
const graphUrls = import.meta.glob('../../data/*/graph.json', { query: '?url', import: 'default', eager: true }) as Record<string, string>;
const placesUrls = import.meta.glob('../../data/*/places.json', { query: '?url', import: 'default', eager: true }) as Record<string, string>;
const datasets: Record<string, { graph: string; places: string }> = {};
for (const [path, url] of Object.entries(graphUrls)) {
  const region = path.split('/').at(-2)!;
  const places = placesUrls[path.replace('graph.json', 'places.json')];
  if (places) datasets[region] = { graph: url, places };
}

// ---------- state ----------
const params = new URLSearchParams(location.search);
const S = {
  engine: null as Engine | null,
  graph: null as Graph | null,
  places: [] as Place[],
  field: null as Field | null, // kept alive: view_buffers reads it on every animation frame
  presets: [] as Preset[],
  features: [] as Feature[],
  presetId: params.get('preset') ?? 'walk',
  metric: { mode: 'foot', weights: { time: 1 }, forbid: [] } as MetricSpec,
  origin: 0,
  t: Number(params.get('t') ?? 720),
  budgetMin: 15,
  alpha: 0,
  category: 'all',
  view: (params.get('view') as View) ?? 'map',
  blend: { warp: 0, lift: 0, metresPerSecond: 1.34 } as ViewBlend,
  nodeCost: new Float32Array(0) as Float32Array<ArrayBufferLike>,
  placeCost: new Float32Array(0) as Float32Array<ArrayBufferLike>,
  fieldMs: 0,
  highlight: -1,
};

(window as unknown as { equid: typeof S }).equid = S; // handy in the devtools console
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const speed = (m: MetricSpec) => (m.mode === 'bike' ? 4.44 : 1.34);
const fmtMin = (s: number) => (Number.isFinite(s) ? (s / 60 < 10 ? (s / 60).toFixed(1) : Math.round(s / 60).toString()) : '∞');
const fmtTime = (t: number) => `${String(Math.floor(t / 60) % 24).padStart(2, '0')}:${String(t % 60).padStart(2, '0')}`;

// ---------- map + deck ----------
maplibregl.setWorkerUrl(mapWorkerUrl);
// Basemap: OpenFreeMap 'positron' (free OSM vector tiles, no key)
const map = new maplibregl.Map({
  container: 'map',
  style: 'https://tiles.openfreemap.org/styles/positron',
  center: [-74.66, 40.349], zoom: 16, maxPitch: 75,
});
const overlay = new MapboxOverlay({
  interleaved: false,
  layers: [],
  onClick: (info: PickingInfo) => onMapClick(info),
  getTooltip: (info: PickingInfo) => tooltip(info),
});
map.addControl(overlay as unknown as maplibregl.IControl);
map.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), 'top-right');

// ---------- engine ----------
function metricForEngine(): string {
  return JSON.stringify(S.metric);
}

let fieldQueued = false;
/** Coalesce a burst of slider events into one engine call. (setTimeout, not requestAnimationFrame:
 * rAF pauses in background/headless tabs and would leave the request stuck.) */
function requestField() {
  if (fieldQueued) return;
  fieldQueued = true;
  setTimeout(() => {
    fieldQueued = false;
    runField();
  }, 0);
}

function runField() {
  if (!S.engine) return;
  const t0 = performance.now();
  try {
    const f = S.engine.field(S.origin, metricForEngine(), S.t);
    S.nodeCost = f.node_cost;
    S.placeCost = f.place_cost;
    S.field?.free();
    S.field = f;
  } catch (e) {
    $('status').textContent = `engine error: ${e}`;
    return;
  }
  S.fieldMs = performance.now() - t0;
  S.blend.metresPerSecond = speed(S.metric);
  render();
}

// ---------- rendering ----------
function originNode(): GNode {
  return S.graph!.nodes[S.origin];
}

function ringStep(budgetMin: number): number {
  for (const s of [1, 2, 5, 10, 15, 20, 30]) if (budgetMin / s <= 6) return s;
  return 30;
}

/** Flat [x0,y0,x1,y1,…] offsets → [[x,y],…] (deck path format). */
const pairs = (a: Float32Array): [number, number][] => Array.from({ length: a.length / 2 }, (_, i) => [a[2 * i], a[2 * i + 1]]);

// The per-vertex math (warp positions, 3D height, colours) runs in the engine (view_buffers); this
// function only hands the typed arrays to deck.gl. Positions are offsets from (origin_lon, origin_lat)
// in degrees, hence COORDINATE_SYSTEM.LNGLAT_OFFSETS on every layer.
function layers() {
  if (!S.graph || !S.engine || !S.field) return [];
  const budgetS = S.budgetMin * 60;
  const v = S.blend;
  const vb = S.engine.view_buffers(S.field, S.origin, v.warp, v.lift, v.metresPerSecond, budgetS);
  const positions = vb.positions, colors = vb.colors, startIndices = vb.start_indices;
  const pathEdge = vb.path_edge, placePos = vb.place_positions;
  const coordinateOrigin: [number, number, number] = [vb.origin_lon, vb.origin_lat, 0];
  vb.free();
  const geo = { coordinateSystem: COORDINATE_SYSTEM.LNGLAT_OFFSETS, coordinateOrigin };

  const out: unknown[] = [];
  if (v.warp > 0.01) {
    const step = ringStep(S.budgetMin);
    const rings = [];
    for (let m = step; m <= S.budgetMin + 1e-9; m += step) rings.push({ m, path: pairs(S.engine.ring(S.origin, m * 60 * v.metresPerSecond)) });
    const a = Math.round(255 * v.warp);
    out.push(new PathLayer({
      id: 'rings', data: rings, ...geo, getPath: (d: { path: [number, number][] }) => d.path,
      getColor: [11, 107, 203, Math.round(a * 0.6)], getWidth: 1.5, widthUnits: 'pixels',
    }));
    out.push(new TextLayer({
      id: 'ring-labels', data: rings, ...geo,
      getPosition: (d: { path: [number, number][] }) => d.path[0],
      getText: (d: { m: number }) => `${d.m} min for you`,
      getSize: 12, getColor: [11, 107, 203, a], getPixelOffset: [0, -8],
      fontFamily: 'system-ui, sans-serif',
    }));
  }

  out.push(new PathLayer({
    id: 'streets', ...geo, pickable: true,
    data: {
      length: startIndices.length - 1,
      startIndices,
      attributes: { getPath: { value: positions, size: 3 }, getColor: { value: colors, size: 4, normalized: true } },
      pathEdge, // read back in tooltip / click via info.index
    },
    _pathType: 'open',
    getWidth: 4, widthUnits: 'pixels', widthMinPixels: 2, capRounded: true, jointRounded: true,
    billboard: true,
  }));

  const placeData = S.places.map((p, i) => ({ p, i, c: S.placeCost[i], pos: [placePos[3 * i], placePos[3 * i + 1], placePos[3 * i + 2]] }))
    .filter(d => Number.isFinite(d.c) && (S.category === 'all' || d.p.category === S.category));
  out.push(new ScatterplotLayer({
    id: 'places', data: placeData, ...geo, pickable: true,
    getPosition: (d: { pos: [number, number, number] }) => d.pos,
    getRadius: (d: { i: number }) => (d.i === S.highlight ? 9 : 5), radiusUnits: 'pixels',
    getFillColor: (d: { c: number }) => (d.c <= budgetS ? [124, 58, 237, 230] : [160, 160, 170, 150]),
    getLineColor: [255, 255, 255], lineWidthMinPixels: 1.5, stroked: true, billboard: true,
    updateTriggers: { getRadius: S.highlight, getFillColor: S.budgetMin },
  }));

  const o = originNode();
  out.push(new ScatterplotLayer({
    id: 'origin', data: [o], ...geo,
    getPosition: (n: GNode) => [n.lon - coordinateOrigin[0], n.lat - coordinateOrigin[1], 0], getRadius: 9, radiusUnits: 'pixels',
    getFillColor: [11, 107, 203], getLineColor: [255, 255, 255], lineWidthMinPixels: 3, stroked: true,
  }));
  return out;
}

/** Graph edge under the cursor for a picked street path, or null. */
function pickedEdge(info: PickingInfo): GEdge | null {
  if (info.layer?.id !== 'streets' || info.index < 0) return null;
  const pathEdge = (info.layer.props.data as { pathEdge: Uint32Array }).pathEdge;
  return S.graph!.edges[pathEdge[info.index]] ?? null;
}

function render() {
  overlay.setProps({ layers: layers() as never });
  renderList();
  renderFormula();
  renderLegend();
  const g = S.graph!;
  $('status').textContent = `${g.meta.region}: ${g.nodes.length} nodes · ${g.edges.length} edges · ${S.places.length} places · field ${S.fieldMs.toFixed(1)} ms`;
}

/** Huff-style ranking: D = cost / worth^α. Worth = smoothed rating / 5 (0.6 if unrated). */
function worth(p: Place): number {
  if (p.rating == null) return 0.6;
  const C = 20, m0 = 3.8, n = p.n ?? 0;
  return Math.max(0.05, ((C * m0 + n * p.rating) / (C + n)) / 5);
}

function renderList() {
  const budgetS = S.budgetMin * 60;
  const rows = S.places.map((p, i) => ({ p, i, c: S.placeCost[i], d: S.placeCost[i] / Math.pow(worth(p), S.alpha) }))
    .filter(r => Number.isFinite(r.c) && (S.category === 'all' || r.p.category === S.category))
    .sort((a, b) => a.d - b.d)
    .slice(0, 50);
  $('places').innerHTML = rows.map(r =>
    `<li data-i="${r.i}" class="${r.c > budgetS ? 'out' : ''}">${r.p.name ?? r.p.category}` +
    `${r.p.rating != null ? ` <small>★${r.p.rating}</small>` : ''}<span class="c">${fmtMin(r.c)} min</span></li>`).join('')
    || '<li class="out">nothing reachable</li>';
}

function renderFormula() {
  const terms = Object.entries(S.metric.weights).filter(([, a]) => a > 0).map(([id, a]) => `${+a.toFixed(2)}·${id}`);
  let s = `edge cost = ${terms.join(' + ') || '0'}`;
  if (S.metric.forbid.length) s += `<br>forbidden: ${S.metric.forbid.join(', ')}`;
  if (S.alpha > 0) s += `<br>place distance = cost / worth^${S.alpha}`;
  $('formula').innerHTML = s;
}

function renderLegend() {
  const extra = S.view === 'warp' ? `rings every ${ringStep(S.budgetMin)} min · radius = your cost`
    : S.view === '3d' ? 'height = your cost' : 'grey = beyond budget';
  $('legend').innerHTML = `<div>your cost (${S.metric.mode === 'bike' ? 'cycling' : 'walking'} minutes-equivalent)</div>` +
    `<div class="bar"></div><div class="ends"><span>0</span><span>${S.budgetMin} min</span></div><div>${extra}</div>`;
}

function tooltip(info: PickingInfo): string | null {
  const e = pickedEdge(info);
  if (e) {
    const c = Math.min(S.nodeCost[e.from], S.nodeCost[e.to]);
    return `${e.highway}${e.steps ? ' (stairs)' : ''} · ${fmtMin(c)} min for you`;
  }
  const o = info.object as { p?: Place; c?: number } | null;
  if (o?.p) return `${o.p.name ?? o.p.category} (${o.p.category}) · ${fmtMin(o.c!)} min for you`;
  return null;
}

// ---------- views (animated blend) ----------
/** Camera for a view: the whole region for the map, the budget circle around you for warp and 3D. */
function fitView(view: View, duration: number) {
  if (!S.graph) return;
  let [w, s, e, n] = S.graph.meta.bbox;
  if (view !== 'map') {
    const o = originNode(), r = S.budgetMin * 60 * speed(S.metric);
    const dLat = r / 110_540, dLon = r / (111_320 * Math.cos((o.lat * Math.PI) / 180));
    [w, s, e, n] = [o.lon - dLon, o.lat - dLat, o.lon + dLon, o.lat + dLat];
  }
  map.fitBounds([[w, s], [e, n]], { padding: view === '3d' ? { top: 160, bottom: 20, left: 40, right: 40 } : 60, duration, pitch: view === '3d' ? 55 : 0, bearing: view === '3d' ? -20 : 0 });
}
let anim = 0;
function setView(view: View, animate = true, refit = true) {
  S.view = view;
  document.querySelectorAll<HTMLButtonElement>('#views button').forEach(b => b.classList.toggle('on', b.dataset.view === view));
  const target = { warp: view === 'warp' ? 1 : 0, lift: view === '3d' ? 0.3 * speed(S.metric) : 0 };
  if (refit) fitView(view, animate ? 800 : 0);
  // warp positions are not geographic, so the basemap fades out
  map.getCanvas().style.opacity = view === 'warp' ? '0.12' : '1';
  const from = { warp: S.blend.warp, lift: S.blend.lift };
  const t0 = performance.now(), dur = animate ? 800 : 0, id = ++anim;
  const step = () => {
    if (id !== anim) return;
    const x = dur ? Math.min(1, (performance.now() - t0) / dur) : 1;
    const e = x < 0.5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2; // ease in-out
    S.blend.warp = from.warp + (target.warp - from.warp) * e;
    S.blend.lift = from.lift + (target.lift - from.lift) * e;
    if (S.graph) overlay.setProps({ layers: layers() as never });
    renderLegend();
    if (x < 1) requestAnimationFrame(step);
  };
  step();
}

function onMapClick(info: PickingInfo) {
  if (!S.engine || !S.graph) return;
  const obj = info.object as { p?: Place } | undefined;
  const edge = pickedEdge(info);
  if (S.view === 'warp') {
    // warped positions are not geographic, so snap to what was clicked (the edge's far end) instead of the coordinate
    if (edge) S.origin = S.nodeCost[edge.from] <= S.nodeCost[edge.to] ? edge.to : edge.from;
    else if (obj?.p) S.origin = obj.p.node;
    else return;
  } else if (info.coordinate) {
    S.origin = S.engine.nearest_node(info.coordinate[0], info.coordinate[1]);
  }
  requestField();
}

// ---------- controls ----------
function applyPreset(id: string) {
  const p = S.presets.find(x => x.id === id);
  if (!p) return;
  S.presetId = id;
  S.metric = { mode: p.mode ?? 'foot', weights: { ...p.weights }, forbid: [...(p.forbid ?? [])] };
  $<HTMLSelectElement>('preset').value = id;
  $('presetDesc').textContent = p.description;
  buildWeightSliders();
  if (S.view === '3d') setView('3d', false, false);
  requestField();
}

function buildWeightSliders() {
  $('weights').innerHTML = `<div class="w"><span>mode</span><select id="mode"><option value="foot">foot</option><option value="bike">bike</option></select><span></span><span></span></div>` +
    S.features.map(f => {
      const a = S.metric.weights[f.id] ?? 0, max = f.id === 'time' ? 5 : 30;
      const forbid = S.metric.forbid.includes(f.id);
      return `<div class="w${forbid ? ' forbid' : ''}" title="${f.desc} (${f.unit})"><span>${f.id}</span>` +
        `<input type="range" min="0" max="${max}" step="0.5" value="${a}" data-f="${f.id}">` +
        `<span class="num">${a}</span><input type="checkbox" title="forbid: edges with any ${f.id} are impassable" data-forbid="${f.id}"${forbid ? ' checked' : ''}></div>`;
    }).join('');
  $<HTMLSelectElement>('mode').value = S.metric.mode;
}

function markCustom() {
  S.presetId = 'custom';
  $<HTMLSelectElement>('preset').value = 'custom';
  $('presetDesc').textContent = 'Your own distance: drag the weights.';
}

function wireControls() {
  $('views').addEventListener('click', e => {
    const v = (e.target as HTMLElement).dataset.view as View | undefined;
    if (v) setView(v);
  });
  $<HTMLSelectElement>('preset').onchange = e => applyPreset((e.target as HTMLSelectElement).value);
  $('weights').addEventListener('input', e => {
    const el = e.target as HTMLInputElement;
    if (el.dataset.f) {
      S.metric.weights[el.dataset.f] = Number(el.value);
      (el.nextElementSibling as HTMLElement).textContent = el.value;
    } else if (el.dataset.forbid) {
      const id = el.dataset.forbid;
      S.metric.forbid = el.checked ? [...S.metric.forbid, id] : S.metric.forbid.filter(x => x !== id);
      el.parentElement!.classList.toggle('forbid', el.checked);
    } else if (el.id === 'mode') {
      S.metric.mode = (el as unknown as HTMLSelectElement).value as 'foot' | 'bike';
    }
    markCustom();
    requestField();
  });
  const time = $<HTMLInputElement>('time');
  time.value = String(S.t);
  time.oninput = () => { S.t = Number(time.value); updateTimeLabel(); requestField(); };
  let playing = 0;
  $('play').onclick = () => {
    if (playing) { clearInterval(playing); playing = 0; $('play').textContent = '▶'; return; }
    if (S.t >= 1425) S.t = 720;
    $('play').textContent = '⏸';
    playing = window.setInterval(() => {
      S.t = Math.min(1425, S.t + 15);
      time.value = String(S.t);
      updateTimeLabel();
      requestField();
      if (S.t >= 1425) { clearInterval(playing); playing = 0; $('play').textContent = '▶'; }
    }, 200);
  };
  const budget = $<HTMLInputElement>('budget');
  budget.oninput = () => { S.budgetMin = Number(budget.value); $('budgetLabel').textContent = `${S.budgetMin} min`; render(); };
  const alpha = $<HTMLInputElement>('alpha');
  alpha.oninput = () => { S.alpha = Number(alpha.value); $('alphaLabel').textContent = S.alpha.toFixed(1); renderList(); renderFormula(); };
  $<HTMLSelectElement>('category').onchange = e => { S.category = (e.target as HTMLSelectElement).value; render(); };
  $('places').addEventListener('click', e => {
    const li = (e.target as HTMLElement).closest('li');
    if (!li?.dataset.i) return;
    S.highlight = Number(li.dataset.i);
    const p = S.places[S.highlight];
    if (S.view !== 'warp') map.easeTo({ center: [p.lon, p.lat], duration: 600 });
    render();
  });
  const ds = $<HTMLSelectElement>('dataset');
  ds.onchange = () => loadDataset(ds.value);
}

function updateTimeLabel() {
  const m = S.graph?.meta;
  const night = m ? S.t < m.sunrise_min || S.t >= m.sunset_min : false;
  $('timeLabel').textContent = `${fmtTime(S.t)} ${night ? '🌙 night' : '☀ day'}`;
}

// ---------- loading ----------
async function loadDataset(region: string) {
  const d = datasets[region];
  $('status').textContent = `loading ${region}…`;
  const [gText, pText] = await Promise.all([fetch(d.graph).then(r => r.text()), fetch(d.places).then(r => r.text())]);
  S.engine?.free();
  S.engine = new Engine(gText, pText);
  S.graph = JSON.parse(gText) as Graph;
  S.places = JSON.parse(pText).places;
  S.highlight = -1;
  const [w, s, e, n] = S.graph.meta.bbox;
  S.origin = params.has('origin') ? Number(params.get('origin')) : S.engine.nearest_node((w + e) / 2, (s + n) / 2);
  S.budgetMin = region === 'fixtures' ? 4 : 15;
  $<HTMLInputElement>('budget').value = String(S.budgetMin);
  $('budgetLabel').textContent = `${S.budgetMin} min`;
  map.fitBounds([[w, s], [e, n]], { padding: 40, duration: 0 });

  const counts = new Map<string, number>();
  for (const p of S.places) counts.set(p.category, (counts.get(p.category) ?? 0) + 1);
  $('category').innerHTML = `<option value="all">all (${S.places.length})</option>` +
    [...counts].sort((a, b) => b[1] - a[1]).map(([c, k]) => `<option value="${c}">${c} (${k})</option>`).join('');
  S.category = 'all';
  updateTimeLabel();
  applyPreset(S.presets.some(p => p.id === S.presetId) ? S.presetId : 'walk');
}

async function main() {
  await init();
  S.presets = JSON.parse(Engine.presets_json());
  S.features = JSON.parse(Engine.features_json());
  $('preset').innerHTML = S.presets.map(p => `<option value="${p.id}">${p.name}</option>`).join('') + '<option value="custom">Custom</option>';
  const regions = Object.keys(datasets).sort((a, b) => (a === 'fixtures' ? 1 : 0) - (b === 'fixtures' ? 1 : 0));
  $('dataset').innerHTML = regions.map(r => `<option value="${r}">${r}</option>`).join('');
  const region = params.get('data') && datasets[params.get('data')!] ? params.get('data')! : regions[0];
  $<HTMLSelectElement>('dataset').value = region;
  $('alphaLabel').textContent = '0.0';
  wireControls();
  await loadDataset(region);
  setView(S.view, false);
}

main().catch(err => { $('status').textContent = `error: ${err}`; console.error(err); });
