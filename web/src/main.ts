// Scaffold page: loads the engine (WASM), the fixture graph + places, calls `field` and draws the result.
// Real map / warp / 3D views come in task 004; this only proves the pipeline end to end.
import init, { Engine } from './pkg/engine.js';
import graphUrl from '../../data/fixtures/graph.json?url';
import placesUrl from '../../data/fixtures/places.json?url';

type Node = { lon: number; lat: number; elev_m: number };
type Edge = { from: number; to: number; geom: [number, number, number][] };
type Graph = { meta: { bbox: [number, number, number, number] }; nodes: Node[]; edges: Edge[] };
type Place = { id: string; name: string | null; category: string; lon: number; lat: number };
type Preset = { id: string; name: string; mode: string; weights: Record<string, number>; forbid?: string[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function main() {
  await init();
  const [graphText, placesText] = await Promise.all([fetch(graphUrl).then(r => r.text()), fetch(placesUrl).then(r => r.text())]);
  const engine = new Engine(graphText, placesText);
  const graph: Graph = JSON.parse(graphText);
  const places: Place[] = JSON.parse(placesText).places;
  const presets: Preset[] = JSON.parse(Engine.presets_json());

  const sel = $<HTMLSelectElement>('preset');
  sel.innerHTML = presets.map(p => `<option value="${p.id}">${p.name}</option>`).join('');
  const time = $<HTMLInputElement>('time');
  const svg = $<HTMLElement>('map') as unknown as SVGSVGElement;
  const [w, s, e, n] = graph.meta.bbox;
  const W = svg.width.baseVal.value, H = svg.height.baseVal.value, pad = 30;
  const x = (lon: number) => pad + ((lon - w) / (e - w)) * (W - 2 * pad);
  const y = (lat: number) => H - pad - ((lat - s) / (n - s)) * (H - 2 * pad);
  const lonOf = (px: number) => w + ((px - pad) / (W - 2 * pad)) * (e - w);
  const latOf = (py: number) => s + ((H - pad - py) / (H - 2 * pad)) * (n - s);
  let origin = 0;

  function render() {
    const preset = presets.find(p => p.id === sel.value)!;
    const t = Number(time.value);
    $('timeLabel').textContent = `${String(Math.floor(t / 60)).padStart(2, '0')}:${String(t % 60).padStart(2, '0')}`;
    const t0 = performance.now();
    const field = engine.field(origin, JSON.stringify(preset), t);
    const ms = performance.now() - t0;
    const nodeCost = field.node_cost, placeCost = field.place_cost;
    const finite = Array.from(nodeCost).filter(Number.isFinite);
    const max = Math.max(1, ...finite);
    const color = (c: number) => (Number.isFinite(c) ? `hsl(${120 - 120 * (c / max)}, 70%, 40%)` : '#bbb');

    svg.innerHTML =
      graph.edges.map(ed => {
        const c = (nodeCost[ed.from] + nodeCost[ed.to]) / 2;
        const pts = ed.geom.map(([lo, la]) => `${x(lo)},${y(la)}`).join(' ');
        return `<polyline points="${pts}" fill="none" stroke="${color(c)}" stroke-width="4"/>`;
      }).join('') +
      graph.nodes.map((nd, i) =>
        `<circle cx="${x(nd.lon)}" cy="${y(nd.lat)}" r="${i === origin ? 8 : 4}" fill="${i === origin ? '#06c' : color(nodeCost[i])}">` +
        `<title>node ${i}: ${(nodeCost[i] / 60).toFixed(1)} min</title></circle>`).join('') +
      places.map(p => `<rect x="${x(p.lon) - 4}" y="${y(p.lat) - 4}" width="8" height="8" fill="#a3c"><title>${p.name ?? p.category}</title></rect>`).join('');

    $('places').innerHTML = '<tr><th>place</th><th>category</th><th>cost (min)</th></tr>' +
      places.map((p, i) => `<tr><td>${p.name ?? '—'}</td><td>${p.category}</td><td>${(placeCost[i] / 60).toFixed(1)}</td></tr>`).join('');
    $('status').textContent = `engine loaded: ${engine.node_count()} nodes, ${engine.place_count()} places · field() took ${ms.toFixed(2)} ms · origin node ${origin}`;
    field.free();
  }

  svg.addEventListener('click', ev => {
    const r = svg.getBoundingClientRect();
    origin = engine.nearest_node(lonOf(ev.clientX - r.left), latOf(ev.clientY - r.top));
    render();
  });
  sel.onchange = render;
  time.oninput = render;
  render();
}

main().catch(err => { $('status').textContent = `error: ${err}`; console.error(err); });
