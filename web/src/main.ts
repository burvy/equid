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
type GEdge = { from: number; to: number; highway: string; steps?: boolean; tree?: number | null; shade?: number[] | null };
type Graph = { meta: { bbox: [number, number, number, number]; region: string; sunrise_min: number; sunset_min: number; slot_min?: number }; nodes: GNode[]; edges: GEdge[] };
/** How far we are between views: 0..1 warp blend, lift = metres of height per second of cost,
 *  hide = 0..1 how far beyond-budget streets/places are put away (on in warp and 3D, off on the map). */
type ViewBlend = { warp: number; lift: number; metresPerSecond: number; hide: number };
type Place = { id: string; name: string | null; category: string; lon: number; lat: number; node: number; rating: number | null; n: number | null;
  tags?: Record<string, string | number> };
type MetricSpec = { mode: 'foot' | 'bike'; weights: Record<string, number>; forbid: string[] };
type Preset = MetricSpec & { id: string; name: string; description: string };
type Feature = { id: string; unit: string; desc: string };
type View = 'map' | 'warp' | '3d';
/** POST /agent response (docs/contracts.md section 6). */
type AgentStop = { id: string; name: string | null; category: string; minutes: number; why: string };
type AgentReply = {
  metric: Partial<MetricSpec> & { weights: Record<string, number> };
  stops: AgentStop[]; pitch: string; llm: string; tools_called?: string[];
  detected?: { label: string; lon: number; lat: number; src?: string }[];
};
/** The last AI answer: stops resolved to indices into S.places (-1 = not in this dataset). */
type AiTrip = { text: string; reply: AgentReply; stopIdx: number[] };

// ---------- datasets: every data/<region>/ with graph.json + places.json ----------
const graphUrls = import.meta.glob('../../data/*/graph.json', { query: '?url', import: 'default', eager: true }) as Record<string, string>;
const placesUrls = import.meta.glob('../../data/*/places.json', { query: '?url', import: 'default', eager: true }) as Record<string, string>;
const objectsUrls = import.meta.glob('../../data/*/objects.json', { query: '?url', import: 'default', eager: true }) as Record<string, string>;
const datasets: Record<string, { graph: string; places: string; objects?: string }> = {};
for (const [path, url] of Object.entries(graphUrls)) {
  const region = path.split('/').at(-2)!;
  const places = placesUrls[path.replace('graph.json', 'places.json')];
  if (places) datasets[region] = { graph: url, places, objects: objectsUrls[path.replace('graph.json', 'objects.json')] };
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
  defaultBudgetMin: 15, // per dataset; an AI answer resets the budget to this, then widens it to fit its stops
  alpha: 0,
  category: 'all',
  /** 'Catch the sunset' mode: sunset spots only, each judged against its own sun-until; the budget follows one spot. */
  sunset: params.get('sunset') === '1',
  view: (params.get('view') as View) ?? 'map',
  blend: { warp: 0, lift: 0, metresPerSecond: 1.34, hide: 0 } as ViewBlend,
  nodeCost: new Float32Array(0) as Float32Array<ArrayBufferLike>,
  placeCost: new Float32Array(0) as Float32Array<ArrayBufferLike>,
  fieldMs: 0,
  highlight: -1,
  ai: null as AiTrip | null,
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


// The per-vertex math (warp positions, 3D height, colours) runs in the engine (view_buffers); this
// function only hands the typed arrays to deck.gl. Positions are offsets from (origin_lon, origin_lat)
// in degrees, hence COORDINATE_SYSTEM.LNGLAT_OFFSETS on every layer.
function layers() {
  if (!S.graph || !S.engine || !S.field) return [];
  const budgetS = S.budgetMin * 60;
  const v = S.blend;
  const vb = S.engine.view_buffers(S.field, S.origin, v.warp, v.lift, v.metresPerSecond, budgetS, v.hide);
  const positions = vb.positions, colors = vb.colors, startIndices = vb.start_indices;
  const pathEdge = vb.path_edge, placePos = vb.place_positions;
  const coordinateOrigin: [number, number, number] = [vb.origin_lon, vb.origin_lat, 0];
  vb.free();
  const geo = { coordinateSystem: COORDINATE_SYSTEM.LNGLAT_OFFSETS, coordinateOrigin };

  const out: unknown[] = [];
  // Budget rings = isochrones (engine: where the cost field crosses N min, outermost per bearing). Their points have
  // cost exactly N min and go through the same warp as the streets, so the blobs on the map morph into the warp circles.
  const step = ringStep(S.budgetMin);
  const rings: { m: number; path: Float32Array }[] = [];
  for (let m = step; m <= S.budgetMin + 1e-9; m += step) {
    const path = S.engine.isochrone(S.field, S.origin, m * 60, v.warp, v.lift, v.metresPerSecond);
    if (path.length >= 9) rings.push({ m, path });
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
  out.push(new PathLayer({
    id: 'rings', data: { length: rings.length, startIndices: ringStarts(rings), attributes: { getPath: { value: concat(rings.map(r => r.path)), size: 3 } } },
    ...geo, _pathType: 'open', getColor: [11, 107, 203, 190], getWidth: 2, widthUnits: 'pixels', jointRounded: true,
  }));

  const sunTarget = S.sunset ? sunsetTarget()?.i ?? -1 : -1;
  const sunOk = new Set(S.sunset ? sunRows.filter(r => r.ok).map(r => r.i) : []);
  const sunKey = [...sunOk].join(','); // spot colour = the list's verdict (orange = catchable), never personal cost
  const placeData = S.places.map((p, i) => ({ p, i, c: S.placeCost[i], pos: [placePos[3 * i], placePos[3 * i + 1], placePos[3 * i + 2]] }))
    .filter(d => Number.isFinite(d.c) && (S.category === 'all' || d.p.category === S.category) && (v.hide < 1 || d.c <= budgetS || sunOk.has(d.i)));
  const beyondAlpha = Math.round(150 * (1 - v.hide));
  const inReach = (d: { c: number; i: number; p: Place }) => (S.sunset && d.p.category === 'sunset' ? sunOk.has(d.i) : d.c <= budgetS);
  out.push(new ScatterplotLayer({
    id: 'places', data: placeData, ...geo, pickable: true,
    getPosition: (d: { pos: [number, number, number] }) => d.pos,
    getRadius: (d: { i: number; p: Place }) => (d.i === S.highlight || d.i === sunTarget ? 9 : d.p.category === 'sunset' ? 7 : 5), radiusUnits: 'pixels',
    getFillColor: (d: { c: number; i: number; p: Place }) => (!inReach(d) ? (S.sunset && d.p.category === 'sunset' ? [110, 110, 120, 220] : [160, 160, 170, beyondAlpha]) : d.p.category === 'sunset' ? [234, 88, 12, 240] : [124, 58, 237, 230]),
    getLineColor: (d: { c: number; i: number; p: Place }) => [255, 255, 255, inReach(d) ? 255 : beyondAlpha],
    lineWidthMinPixels: 1.5, stroked: true, billboard: true,
    updateTriggers: { getRadius: [S.highlight, sunTarget], getFillColor: [S.budgetMin, beyondAlpha, S.sunset, S.t, sunKey], getLineColor: [S.budgetMin, beyondAlpha, S.sunset, S.t, sunKey] },
  }));

  // the AI's curated stops: numbered amber markers on top of the places, in every view
  const stops = (S.ai?.stopIdx ?? []).map((i, k) => ({ k: k + 1, i, pos: [placePos[3 * i], placePos[3 * i + 1], placePos[3 * i + 2]] }))
    .filter(d => d.i >= 0);
  if (stops.length) {
    out.push(new ScatterplotLayer({
      id: 'stops', data: stops, ...geo, pickable: true,
      getPosition: (d: { pos: [number, number, number] }) => d.pos,
      getRadius: (d: { i: number }) => (d.i === S.highlight ? 14 : 11), radiusUnits: 'pixels',
      getFillColor: [245, 158, 11], getLineColor: [255, 255, 255], lineWidthMinPixels: 2.5, stroked: true, billboard: true,
      updateTriggers: { getRadius: S.highlight },
    }));
    out.push(new TextLayer({
      id: 'stop-numbers', data: stops, ...geo,
      getPosition: (d: { pos: [number, number, number] }) => d.pos,
      getText: (d: { k: number }) => String(d.k), getSize: 12, getColor: [255, 255, 255],
      fontFamily: 'system-ui, sans-serif', fontWeight: 700, billboard: true,
    }));
  }

  const o = originNode();
  out.push(new ScatterplotLayer({
    id: 'origin', data: [o], ...geo,
    getPosition: (n: GNode) => [n.lon - coordinateOrigin[0], n.lat - coordinateOrigin[1], 0], getRadius: 9, radiusUnits: 'pixels',
    getFillColor: [11, 107, 203], getLineColor: [255, 255, 255], lineWidthMinPixels: 3, stroked: true,
  }));
  // ring labels last, with a white halo, so streets and places never cover them; at the ring's northmost-bearing point
  out.push(new TextLayer({
    id: 'ring-labels', data: rings, ...geo,
    getPosition: (d: { path: Float32Array }) => [d.path[0], d.path[1], d.path[2]],
    getText: (d: { m: number }) => `${d.m} min for you`,
    getSize: 12, getColor: [11, 107, 203], getPixelOffset: [0, -9],
    fontFamily: 'system-ui, sans-serif', fontWeight: 600,
    fontSettings: { sdf: true }, outlineWidth: 4, outlineColor: [255, 255, 255, 230],
  }));
  return out;
}

/** deck binary paths: start index (in vertices) of each ring, then the total. */
function ringStarts(rings: { path: Float32Array }[]): Uint32Array {
  const out = new Uint32Array(rings.length + 1);
  rings.forEach((r, i) => { out[i + 1] = out[i] + r.path.length / 3; });
  return out;
}
const concat = (parts: Float32Array[]) => {
  const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
};

/** Graph edge under the cursor for a picked street path, or null. */
function pickedEdge(info: PickingInfo): GEdge | null {
  if (info.layer?.id !== 'streets' || info.index < 0) return null;
  const pathEdge = (info.layer.props.data as { pathEdge: Uint32Array }).pathEdge;
  return S.graph!.edges[pathEdge[info.index]] ?? null;
}

function render() {
  if (S.sunset) syncSunset();
  overlay.setProps({ layers: layers() as never });
  renderList();
  renderFormula();
  renderLegend();
  renderAi(); // stop minutes follow the current field
  const g = S.graph!;
  $('status').textContent = `${g.meta.region}: ${g.nodes.length} nodes · ${g.edges.length} edges · ${S.places.length} places · field ${S.fieldMs.toFixed(1)} ms`;
}

/** Huff-style ranking: D = cost / worth^α. Worth = smoothed rating / 5 (0.6 if unrated). */
function worth(p: Place): number {
  if (p.rating == null) return 0.6;
  const C = 20, m0 = 3.8, n = p.n ?? 0;
  return Math.max(0.05, ((C * m0 + n * p.rating) / (C + n)) / 5);
}

// OSM categories as people say them; unnamed places show these instead of 'bicycle_parking'.
const CATEGORY_LABELS: Record<string, string> = {
  artwork: 'Public art', attraction: 'Attraction', bench: 'Bench', bicycle_parking: 'Bike parking', cafe: 'Café',
  drinking_water: 'Drinking fountain', fast_food: 'Fast food', garden: 'Garden', ice_cream: 'Ice cream',
  library: 'Library', museum: 'Museum', nature_reserve: 'Nature reserve', park: 'Park', playground: 'Playground',
  restaurant: 'Restaurant', shelter: 'Shelter', toilets: 'Public toilets', viewpoint: 'Viewpoint', sunset: 'Sunset spot',
};
/** Unknown categories: 'dog_park' -> 'Dog park'. */
const categoryLabel = (c: string) => CATEGORY_LABELS[c] ?? (c.charAt(0).toUpperCase() + c.slice(1)).replace(/_/g, ' ');
const placeLabel = (p: Place) => p.name ?? categoryLabel(p.category);
/** 'sun until 16:35' for places where prep-shade found the low sun visible (sunset spots, some parks), else ''. */
const sunUntil = (p: Place) => (p.tags?.sun_visible_until ? `sun until ${p.tags.sun_visible_until}` : '');

// ---------- 'catch the sunset' ----------
// Every sunset spot has its own deadline: the minute the low sun stops being visible there (prep's sun_visible_until).
// Your personal cost picks the route; the verdict uses the real walking time along that route (its length at the
// mode's base speed), because cost includes penalty seconds (stroller, shade...) that aren't minutes on the clock.
type SunRow = { i: number; p: Place; c: number; w: number; left: number; ok: boolean };
const hhmmMin = (s?: string | number) => { const m = /^(\d{1,2}):(\d{2})$/.exec(String(s ?? '')); return m ? +m[1] * 60 + +m[2] : NaN; };
const hasSunsetSpots = () => S.places.some(p => p.category === 'sunset' && Number.isFinite(hhmmMin(p.tags?.sun_visible_until)));
let sunRows: SunRow[] = [];
/** Real walking minutes to each sunset spot (by place index), for the field they were computed under. */
let sunWalk: { field: unknown; min: Map<number, number> } = { field: null, min: new Map() };
/** Naismith's rule: walking takes an extra ~6 s per metre climbed (1 h per 600 m). Rides stay length / speed. */
const NAISMITH_S_PER_M = 6;

/** One engine.route_walk call per new field (origin, metric or time changed): ~8 spots, cheap.
 *  Minutes = length / base speed + (on foot) 6 s per metre climbed, on the route your preferences pick. */
function sunsetWalkMinutes(idx: number[]): Map<number, number> {
  if (sunWalk.field === S.field && idx.every(i => sunWalk.min.has(i))) return sunWalk.min;
  const min = new Map<number, number>();
  if (S.engine && idx.length) {
    try {
      const lc = S.engine.route_walk(S.origin, metricForEngine(), S.t, Uint32Array.from(idx, i => S.places[i].node));
      const climbS = S.metric.mode === 'bike' ? 0 : NAISMITH_S_PER_M;
      idx.forEach((i, k) => min.set(i, (lc[2 * k] / speed(S.metric) + climbS * lc[2 * k + 1]) / 60));
    } catch (e) { console.warn('route_walk:', e); }
  }
  sunWalk = { field: S.field, min };
  return min;
}

/** Sunset spots with minutes left, real walking minutes and personal cost, most time to spare first. */
function sunsetRows(): SunRow[] {
  const spots = S.places.flatMap((p, i) => (p.category === 'sunset' && Number.isFinite(hhmmMin(p.tags?.sun_visible_until)) ? [i] : []));
  const walk = sunsetWalkMinutes(spots.filter(i => Number.isFinite(S.placeCost[i])));
  return spots.map(i => {
    const p = S.places[i], c = S.placeCost[i] / 60, left = hhmmMin(p.tags?.sun_visible_until) - S.t;
    const w = Number.isFinite(c) ? walk.get(i) ?? NaN : NaN;
    return { i, p, c, w, left, ok: Number.isFinite(w) && left > 0 && w <= left };
  }).sort((a, b) => (b.left - b.w) - (a.left - a.w) || b.left - a.left);
}

/** The spot the budget follows: the one picked in the list or on the map, else the catchable one with the most time to spare. */
function sunsetTarget(): SunRow | undefined {
  return sunRows.find(r => r.i === S.highlight) ?? sunRows.find(r => r.ok);
}

/** Budget = minutes left before the target spot loses the sun (so the rings show exactly how far you can still go). */
function syncSunset() {
  sunRows = sunsetRows();
  const tgt = sunsetTarget();
  const b = tgt ? Math.min(60, Math.max(1, Math.floor(tgt.left))) : S.defaultBudgetMin;
  if (b !== S.budgetMin) {
    S.budgetMin = b;
    $<HTMLInputElement>('budget').value = String(b);
    $('budgetLabel').textContent = `${b} min`;
  }
}

const spotName = (p: Place) => placeLabel(p).replace(/^Sunset spot: /, '');
function sunVerdict(r: SunRow): string {
  if (r.left <= 0) return 'sun already gone';
  if (!Number.isFinite(r.w)) return 'can\'t get there';
  return r.ok ? `✓ catch it, ${fmtMin((r.left - r.w) * 60)} min to spare` : `too late by ${fmtMin((r.w - r.left) * 60)} min`;
}
/** '12 min walk', plus your cost when penalties make it differ: '12 min walk · costs you 19'. */
const sunWalkLabel = (r: SunRow) => (!Number.isFinite(r.w) ? '' :
  `${fmtMin(r.w * 60)} min ${S.metric.mode === 'bike' ? 'ride' : 'walk'}` +
  (fmtMin(r.c * 60) !== fmtMin(r.w * 60) ?` <small title="your personal cost (picks the route, includes penalties)">· costs you ${fmtMin(r.c * 60)}</small>` : ''));

function renderSunsetList() {
  const tgt = sunsetTarget(), n = sunRows.filter(r => r.ok).length;
  $('sunsetNote').innerHTML = (n
    ? `At ${fmtTime(S.t)} you can still catch <b>${n} of ${sunRows.length}</b> sunset spots.`
    : `At ${fmtTime(S.t)} it's too late for every sunset spot: try an earlier time.`) +
    (tgt ? ` Budget = <b>${S.budgetMin} min</b>, until the sun leaves <b>${esc(spotName(tgt.p))}</b> at ${tgt.p.tags?.sun_visible_until}.` : '') +
    (S.metric.mode === 'bike'
      ? ' <small>Judged on riding time along your route, on flat ground (hills not counted; your preferences pick the route).</small>'
      : ' <small>Judged on walking time along your route, plus 6 s per metre climbed (Naismith); your preferences pick the route.</small>');
  $('places').innerHTML = sunRows.map(r =>
    `<li data-i="${r.i}" class="${r.ok ? '' : 'out'}${r === tgt ? ' target' : ''}">${esc(spotName(r.p))}` +
    ` <small class="sun">☀ until ${r.p.tags?.sun_visible_until}</small><span class="c">${sunWalkLabel(r)}</span>` +
    `<div class="verdict ${r.ok ? 'ok' : 'late'}">${sunVerdict(r)}</div></li>`).join('');
}

/** Turn the mode on/off: on = sunset spots only and a budget that follows them; off = back to all places and the usual budget. */
function setSunset(on: boolean, draw = true) {
  S.sunset = on && hasSunsetSpots();
  $('sunsetMode').classList.toggle('on', S.sunset);
  if (S.sunset) S.category = 'sunset';
  else {
    $('sunsetNote').innerHTML = '';
    if (S.category === 'sunset') S.category = 'all';
    S.budgetMin = S.defaultBudgetMin;
    $<HTMLInputElement>('budget').value = String(S.budgetMin);
    $('budgetLabel').textContent = `${S.budgetMin} min`;
  }
  $<HTMLSelectElement>('category').value = S.category;
  if (draw && S.field) render();
}

function renderList() {
  if (S.sunset) return renderSunsetList();
  const budgetS = S.budgetMin * 60;
  const rows = S.places.map((p, i) => ({ p, i, c: S.placeCost[i], d: S.placeCost[i] / Math.pow(worth(p), S.alpha) }))
    .filter(r => Number.isFinite(r.c) && (S.category === 'all' || r.p.category === S.category))
    .sort((a, b) => a.d - b.d)
    .slice(0, 50);
  $('places').innerHTML = rows.map(r =>
    `<li data-i="${r.i}" class="${r.c > budgetS ? 'out' : ''}">${placeLabel(r.p)}` +
    `${r.p.rating != null ? ` <small>★${r.p.rating}</small>` : ''}${sunUntil(r.p) ? ` <small class="sun">☀ ${sunUntil(r.p)}</small>` : ''}<span class="c">${fmtMin(r.c)} min</span></li>`).join('')
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
  const rings = `rings every ${ringStep(S.budgetMin)} min`;
  const extra = S.view === 'warp' ? `${rings} · radius = your cost`
    : S.view === '3d' ? `height = your cost · ${rings}` : `${rings} (isochrones) · grey = beyond budget`;
  $('legend').innerHTML = `<div>your cost (${S.metric.mode === 'bike' ? 'cycling' : 'walking'} minutes-equivalent)</div>` +
    `<div class="bar"></div><div class="ends"><span>0</span><span>${S.budgetMin} min</span></div><div>${extra}</div>`;
}

/** ' · 70% shade now · trees 40%' from the edge's LiDAR shade slots (current time) and tree cover, when present. */
function shadeText(e: GEdge): string {
  const m = S.graph!.meta, parts: string[] = [];
  const night = S.t < m.sunrise_min || S.t >= m.sunset_min;
  if (e.shade?.length && !night) parts.push(`${Math.round(100 * e.shade[Math.floor(S.t / (m.slot_min ?? 30)) % e.shade.length])}% shade now`);
  if (e.tree != null) parts.push(`trees ${Math.round(100 * e.tree)}%`);
  return parts.map(x => ` · ${x}`).join('');
}

function tooltip(info: PickingInfo): string | null {
  const e = pickedEdge(info);
  if (e) {
    const c = Math.min(S.nodeCost[e.from], S.nodeCost[e.to]);
    return `${e.highway}${e.steps ? ' (stairs)' : ''} · ${fmtMin(c)} min for you${shadeText(e)}`;
  }
  const o = info.object as { p?: Place; c?: number; k?: number; i?: number } | null;
  if (o?.k && o.i != null && S.ai) {
    const p = S.places[o.i];
    return `${o.k}. ${placeLabel(p)} · ${fmtMin(S.placeCost[o.i])} min for you\n${tidy(S.ai.reply.stops[o.k - 1]?.why ?? '')}`;
  }
  if (o?.p) return `${placeLabel(o.p)}${o.p.name && o.p.category !== 'sunset' ? ` (${categoryLabel(o.p.category)})` : ''} · ${fmtMin(o.c!)} min for you` +
    `${sunUntil(o.p) ? `
☀ ${sunUntil(o.p)}${o.p.tags?.sunset ? ` (sunset ${o.p.tags.sunset})` : ''}` : ''}` +
    `${S.sunset && o.p.category === 'sunset' ? `\n${sunVerdict(sunRows.find(r => r.p === o.p)!)}` : ''}`;
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
  const target = { warp: view === 'warp' ? 1 : 0, lift: view === '3d' ? 0.3 * speed(S.metric) : 0, hide: view === 'map' ? 0 : 1 };
  if (refit) fitView(view, animate ? 800 : 0);
  // warp positions are not geographic, so the basemap fades out
  map.getCanvas().style.opacity = view === 'warp' ? '0.12' : '1';
  const from = { warp: S.blend.warp, lift: S.blend.lift, hide: S.blend.hide };
  const t0 = performance.now(), dur = animate ? 800 : 0, id = ++anim;
  const step = () => {
    if (id !== anim) return;
    const x = dur ? Math.min(1, (performance.now() - t0) / dur) : 1;
    const e = x < 0.5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2; // ease in-out
    S.blend.warp = from.warp + (target.warp - from.warp) * e;
    S.blend.lift = from.lift + (target.lift - from.lift) * e;
    S.blend.hide = from.hide + (target.hide - from.hide) * e;
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
    else if ((obj as { i?: number } | undefined)?.i != null) S.origin = S.places[(obj as { i: number }).i].node;
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
  // built-in features, plus any other weight the metric uses (the AI can weight near:<label> object terms)
  const extra: Feature[] = Object.keys(S.metric.weights).filter(id => !S.features.some(f => f.id === id))
    .map(id => ({ id, unit: 'object distance', desc: termDesc(id) }));
  $('weights').innerHTML = `<div class="w"><span>mode</span><select id="mode"><option value="foot">foot</option><option value="bike">bike</option></select><span></span><span></span></div>` +
    [...S.features, ...extra].map(f => {
      const a = S.metric.weights[f.id] ?? 0, max = f.id === 'time' ? 5 : 30;
      const forbid = S.metric.forbid.includes(f.id);
      return `<div class="w${forbid ? ' forbid' : ''}" title="${termPhrase(f.id)}. ${f.id}: ${f.desc} (${f.unit})"><span>${termLabel(f.id)}</span>` +
        `<input type="range" min="0" max="${max}" step="0.1" value="${a}" data-f="${f.id}">` +
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
  $<HTMLSelectElement>('preset').onchange = e => {
    const v = (e.target as HTMLSelectElement).value;
    if (v === 'ai' && S.ai) applyAiMetric(); else applyPreset(v);
  };
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
  budget.oninput = () => { if (S.sunset) { S.sunset = false; $('sunsetMode').classList.remove('on'); $('sunsetNote').innerHTML = ''; } S.budgetMin = Number(budget.value); $('budgetLabel').textContent = `${S.budgetMin} min`; render(); };
  const alpha = $<HTMLInputElement>('alpha');
  alpha.oninput = () => { S.alpha = Number(alpha.value); $('alphaLabel').textContent = S.alpha.toFixed(1); renderList(); renderFormula(); };
  $<HTMLSelectElement>('category').onchange = e => {
    S.category = (e.target as HTMLSelectElement).value;
    if (S.sunset && S.category !== 'sunset') setSunset(false, false);
    render();
  };
  $('sunsetMode').onclick = () => { S.highlight = -1; setSunset(!S.sunset); };
  $('places').addEventListener('click', e => {
    const li = (e.target as HTMLElement).closest('li');
    if (!li?.dataset.i) return;
    S.highlight = Number(li.dataset.i);
    const p = S.places[S.highlight];
    if (S.view !== 'warp') map.easeTo({ center: [p.lon, p.lat], duration: 600 });
    render();
  });
  wireAi();
  const ds = $<HTMLSelectElement>('dataset');
  ds.onchange = () => loadDataset(ds.value);
}

function updateTimeLabel() {
  const m = S.graph?.meta;
  const night = m ? S.t < m.sunrise_min || S.t >= m.sunset_min : false;
  $('timeLabel').textContent = `${fmtTime(S.t)} ${night ? '🌙 night' : '☀ day'}`;
}

// ---------- AI trip panel (POST /agent, docs/contracts.md section 6) ----------
// The server is reached on the page's own origin (Vite proxies /agent to cargo run -p server); ?agent=<url> overrides.
const AGENT_URL = params.get('agent') ?? '/agent';
const AGENT_TIMEOUT_MS = 180_000; // real Claude runs a multi-tool loop: tens of seconds is normal
/** f32 noise in model/server text ('0.699999988079071 minutes') -> '0.7'; other numbers untouched. */
const tidy = (s: string) => s.replace(/\d+\.\d{4,}/g, x => String(+Number(x).toFixed(1)));
const esc = (s: string) => s.replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);

/** Precise meaning of one metric term (the engine's formula), shown on hover. */
function termDesc(id: string): string {
  if (id.startsWith('near:')) return `cost grows with distance to the nearest ${id.slice(5)}`;
  return S.features.find(f => f.id === id)?.desc ?? id;
}

/** Per feature: what a positive weight means and what a forbid means (a few words, the formula is on hover),
 * and a short slider label. */
const PHRASE: Record<string, [weight: string, forbid: string, label: string]> = {
  time: ['get there quickly', 'no time limit', 'time'],
  climb: ['avoid climbing', 'no uphill at all', 'climbing'],
  steep: ['avoid steep slopes', 'no steep slopes', 'steep'],
  steps: ['avoid stairs', 'no stairs at all', 'stairs'],
  rough: ['avoid rough surfaces', 'no rough surfaces', 'rough'],
  busy: ['stay off busy roads', 'no busy roads', 'busy roads'],
  unlit: ['stay on lit streets', 'no dark streets', 'dark'],
  sun: ['stay in the shade', 'no sunny stretches', 'sun'],
  exposed: ['stay under cover', 'no uncovered stretches', 'no cover'],
  stress: ['stay on calm bike streets', 'no stressful bike streets', 'bike stress'],
  bench_gap: ['stay near benches', 'no stretches far from a bench', 'far: bench'],
  toilet_gap: ['stay near toilets', 'no stretches far from toilets', 'far: toilet'],
  water_gap: ['stay near drinking water', 'no stretches far from water', 'far: water'],
  // 028 features
  icy: ['avoid icy hills', 'no icy stretches', 'icy'],
  flood: ['avoid flood zones', 'no flood zones', 'flood zone'],
  crossing: ['fewer road crossings', 'no road crossings', 'crossings'],
  no_sidewalk: ['stay on sidewalks', 'no roads without a sidewalk', 'no sidewalk'],
  kerb: ['avoid raised kerbs', 'no raised kerbs', 'kerbs'],
  dull: ['go past greenery and water', 'only green or waterside streets', 'dull streets'],
  stop: ['fewer stop signs and lights', 'no stop signs or lights', 'stops'],
};
const termLabel = (id: string) => id.startsWith('near:') ? `near ${id.slice(5)}` : PHRASE[id]?.[2] ?? id;
function termPhrase(id: string, forbid = false): string {
  if (id.startsWith('near:')) return forbid ? `no ${id.slice(5)} nearby` : `stay near ${id.slice(5)}`;
  return PHRASE[id]?.[forbid ? 1 : 0] ?? (forbid ? `no ${id}` : `avoid ${id}`);
}
/** Weight size in words, so '×0.5' and '×3' read at a glance (time is 1 by convention). */
const strength = (w: number) => w >= 5 ? 'strong' : w >= 1.5 ? 'medium' : 'light';

/** 'Custom (AI)' appears in the preset menu only while there is an AI answer. */
function syncAiPreset() {
  const sel = $<HTMLSelectElement>('preset');
  const opt = sel.querySelector('option[value="ai"]');
  if (S.ai && !opt) sel.insertAdjacentHTML('beforeend', '<option value="ai">Custom (AI)</option>');
  if (!S.ai && opt) opt.remove();
}

/** Put the AI's metric in charge: preset menu, sliders, formula, field, map. */
function applyAiMetric() {
  const m = S.ai!.reply.metric;
  S.presetId = 'ai';
  S.metric = { mode: m.mode ?? 'foot', weights: { ...m.weights }, forbid: [...(m.forbid ?? [])] };
  syncAiPreset();
  $<HTMLSelectElement>('preset').value = 'ai';
  $('presetDesc').textContent = `Weights chosen by the AI for “${S.ai!.text}”.`;
  buildWeightSliders();
  $<HTMLDetailsElement>('weightsBox').open = true; // show the sliders the AI just set
  if (S.view === '3d') setView('3d', false, false);
  runField(); // synchronous, so the budget check below sees the new costs
  // budget = the usual one, widened so every stop is inside it (beyond-budget places are hidden in warp / 3D)
  const worst = Math.max(0, ...S.ai!.stopIdx.filter(i => i >= 0).map(i => S.placeCost[i]).filter(Number.isFinite));
  const fit = Math.min(60, Math.max(S.defaultBudgetMin, Math.ceil(worst / 60) + 1));
  if (fit !== S.budgetMin) {
    S.budgetMin = fit;
    $<HTMLInputElement>('budget').value = String(S.budgetMin);
    $('budgetLabel').textContent = `${S.budgetMin} min`;
    if (S.view !== 'map') fitView(S.view, 600);
    render();
  }
}

/** Pitch, numbered stops (minutes from the current field) and the AI's weights in plain English. */
function renderAi() {
  const a = S.ai;
  if (!a || aiPending) return; // don't overwrite the 'Thinking…' line
  const r = a.reply;
  const stops = r.stops.map((st, k) => {
    const i = a.stopIdx[k];
    const name = esc(st.name ?? categoryLabel(st.category));
    const mins = i >= 0 && Number.isFinite(S.placeCost[i]) ? fmtMin(S.placeCost[i]) : fmtMin(st.minutes * 60);
    return `<li data-i="${i}"><b>${name}</b> · ${mins} min<div class="why">${esc(tidy(st.why))}</div></li>`;
  }).join('');
  // plain phrase first; the feature id, weight and formula are one hover away
  const forbids = r.metric.forbid ?? [];
  const terms = forbids.map(f => `<li title="${esc(f)}: forbidden, any street with it is impassable">` +
    `<b>${esc(termPhrase(f, true))}</b></li>`);
  terms.push(...Object.entries(r.metric.weights).filter(([id, w]) => w > 0 && !forbids.includes(id))
    .sort(([a, x], [b, y]) => (a === 'time' ? 1 : 0) - (b === 'time' ? 1 : 0) || y - x)
    .map(([id, w]) => {
      const s = id === 'time' ? '' : strength(w), wt = +w.toFixed(2);
      return `<li title="${esc(id)} ×${wt}: ${esc(termDesc(id))}"><b>${esc(termPhrase(id))}</b> <span class="wt">${s ? `${s} ` : ''}×${wt}</span></li>`;
    }));
  const missing = a.stopIdx.filter(i => i < 0).length;
  $('aiOut').innerHTML =
    `<p class="pitch">${esc(tidy(r.pitch))}</p>` +
    (r.stops.length ? `<h4>Stops</h4><ol class="stops">${stops}</ol>` : '') +
    (missing ? `<p class="err">${missing} stop(s) aren't in this map's data: is the server on the same dataset (EQUID_DATA)?</p>` : '') +
    `<h4>Why these weights</h4><ul>${terms.join('')}</ul>` +
    `<div class="meta">${r.llm === 'mock' ? 'offline mock model (no API key on the server)' : esc(r.llm)}` +
    `${r.detected?.length ? ` · found ${r.detected.length} object(s) live` : ''}</div>`;
}

let aiAsk = 0, aiPending = false;
async function askAgent(text: string) {
  if (!S.engine || !S.graph || !text.trim()) return;
  const id = ++aiAsk, go = $<HTMLButtonElement>('aiGo');
  go.disabled = true;
  aiPending = true;
  const t0 = performance.now();
  const tick = () => { if (id === aiAsk) $('aiOut').innerHTML = `<p class="busy">Thinking… the agent is picking weights and checking places (${Math.round((performance.now() - t0) / 1000)} s)</p>`; };
  tick();
  const timer = setInterval(tick, 1000);
  const ctl = new AbortController();
  const kill = setTimeout(() => ctl.abort(), AGENT_TIMEOUT_MS);
  const o = originNode();
  try {
    const res = await fetch(AGENT_URL, {
      method: 'POST', headers: { 'content-type': 'application/json' }, signal: ctl.signal,
      body: JSON.stringify({ text, origin: { lon: o.lon, lat: o.lat }, t: S.t }),
    });
    const body = await res.json().catch(() => null);
    if (id !== aiAsk) return; // a newer question was asked meanwhile
    clearInterval(timer);
    aiPending = false;
    if (!res.ok || !body?.metric) {
      // the Vite proxy answers 500/502/504 with no JSON when the server isn't running
      throw new Error(body?.error ?? (res.status >= 500 && !body ? 'unreachable' : `the agent answered HTTP ${res.status}`));
    }
    const reply = body as AgentReply;
    // objects the agent found live (Vexcel) must exist in our engine before its near:<label> weights compile
    const byLabel = new Map<string, [number, number][]>();
    for (const d of reply.detected ?? []) byLabel.set(d.label, [...(byLabel.get(d.label) ?? []), [d.lon, d.lat]]);
    for (const [label, pts] of byLabel) S.engine.add_objects(label, JSON.stringify(pts));
    S.ai = { text, reply, stopIdx: reply.stops.map(st => S.places.findIndex(p => p.id === st.id)) };
    S.highlight = -1;
    applyAiMetric();
    renderAi();
  } catch (e) {
    if (id !== aiAsk) return;
    clearInterval(timer);
    aiPending = false;
    const err = e as Error;
    const msg = err.name === 'AbortError' ? `No answer after ${AGENT_TIMEOUT_MS / 1000} s.`
      : err instanceof TypeError || err.message === 'unreachable'
        ? `Can't reach the trip agent at ${esc(AGENT_URL)}. Start it with <code>cargo run -p server</code>.`
        : `The agent failed: ${esc(err.message)}`;
    $('aiOut').innerHTML = `<p class="err">${msg}</p>`;
  } finally {
    clearInterval(timer);
    clearTimeout(kill);
    if (id === aiAsk) go.disabled = false;
  }
}

function wireAi() {
  const box = $<HTMLTextAreaElement>('aiText');
  $('aiGo').onclick = () => askAgent(box.value);
  box.addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); askAgent(box.value); } });
  $('aiChips').addEventListener('click', e => {
    const b = (e.target as HTMLElement).closest('button');
    if (!b?.dataset.text) return;
    box.value = b.dataset.text;
    // each example is about a time of day: set the clock first so the agent and the field use it
    S.t = Number(b.dataset.t);
    $<HTMLInputElement>('time').value = String(S.t);
    updateTimeLabel();
    askAgent(b.dataset.text);
  });
  $('aiOut').addEventListener('click', e => {
    const li = (e.target as HTMLElement).closest<HTMLElement>('li[data-i]');
    const i = Number(li?.dataset.i ?? -1);
    if (i < 0) return;
    S.highlight = i;
    const p = S.places[i];
    if (S.view === 'map') map.easeTo({ center: [p.lon, p.lat], duration: 600 });
    render();
  });
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
  if (d.objects) {
    // object positions for near:<label> weights (the agent may weight them); optional per dataset
    try { S.engine.load_objects(await fetch(d.objects).then(r => r.text())); } catch (e) { console.warn('objects.json:', e); }
  }
  S.highlight = -1;
  S.ai = null;
  $('aiOut').innerHTML = '';
  syncAiPreset();
  const [w, s, e, n] = S.graph.meta.bbox;
  S.origin = params.has('origin') ? Number(params.get('origin')) : S.engine.nearest_node((w + e) / 2, (s + n) / 2);
  S.budgetMin = S.defaultBudgetMin = region === 'fixtures' ? 4 : 15;
  $<HTMLInputElement>('budget').value = String(S.budgetMin);
  $('budgetLabel').textContent = `${S.budgetMin} min`;
  map.fitBounds([[w, s], [e, n]], { padding: 40, duration: 0 });

  const counts = new Map<string, number>();
  for (const p of S.places) counts.set(p.category, (counts.get(p.category) ?? 0) + 1);
  $('category').innerHTML = `<option value="all">all (${S.places.length})</option>` +
    [...counts].sort((a, b) => b[1] - a[1]).map(([c, k]) => `<option value="${c}">${categoryLabel(c)} (${k})</option>`).join('');
  S.category = 'all';
  $('sunsetMode').hidden = !hasSunsetSpots();
  setSunset(S.sunset, false);
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
