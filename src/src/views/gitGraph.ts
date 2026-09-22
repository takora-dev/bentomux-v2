/* ---------------- commit graph ----------------
   Two halves. `layoutGraph` is pure and DOM-free: it walks the commits in the
   order `git log --date-order` produced them and assigns each one a lane. The
   only invariant it trusts is the one the backend guarantees — no parent is
   listed before all of its children — because a commit's lane can only be
   resolved once every child that points at it has been drawn.

   `graphBody` turns the layout into one SVG (all lane segments, grouped by
   colour) plus the label column. Row height is shared between the two so they
   stay aligned as the panel scrolls. */

import { h } from '../dom';
import { rel } from '../time';
import { go } from '../router';
import type { GitCommit, GitRef } from '../../shared/types';

/* the SVG and the label column must agree on row height, so it lives here */
const ROW_H = 22;
const LANE_W = 10;
const LANE_PAD = 5;
const DOT_R = 3;

/* mid-tone hues, legible on all six themes. The palette has no per-lane
   colour and a graph without lane colour is unreadable. */
const LANE_COLORS = [
  '#4c8dff', '#f0883e', '#3fb950', '#d2a8ff',
  '#ff7b72', '#39c5cf', '#e3b341', '#a5d6ff',
];

interface GraphEdge {
  from: number;
  to: number;
}

interface GraphRow {
  /** the lane this commit's dot sits on */
  lane: number;
  /** a lane was already expecting this commit, so a line comes in from above */
  above: boolean;
  /** lanes unrelated to this commit that stay occupied across the whole row */
  through: number[];
  /** one per parent: the lane this commit's history leaves through */
  edges: GraphEdge[];
}

interface GraphLayout {
  rows: GraphRow[];
  /** widest lane count seen, so the SVG can size its column */
  lanes: number;
}

/** Assign every commit a lane, in the order the backend listed them. */
export function layoutGraph(commits: GitCommit[]): GraphLayout {
  /* each slot holds the oid of a commit still expected to appear further down
     the list; null is a free slot that a later branch tip can claim */
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  let widest = 0;

  for (const commit of commits) {
    let lane = lanes.indexOf(commit.oid);
    if (lane === -1) {
      /* a branch tip: no child has claimed a lane for it yet */
      lane = lanes.indexOf(null);
      if (lane === -1) {
        lane = lanes.length;
        lanes.push(null);
      }
    }
    const above = lanes[lane] !== null;
    const before = lanes.map(v => v !== null);
    lanes[lane] = null;

    const edges: GraphEdge[] = [];
    for (const parent of commit.parents) {
      let to = lanes.indexOf(parent);
      if (to === -1) {
        /* a merge's second parent opens a lane; a plain parent usually reuses
           the one it is already expected on */
        to = lanes.indexOf(null);
        if (to === -1) {
          to = lanes.length;
          lanes.push(null);
        }
      }
      lanes[to] = parent;
      edges.push({ from: lane, to });
    }

    /* a lane occupied before and still occupied after, and not this commit's
       own lane, is an unrelated branch running past this row. A lane that
       closes here (root commit) is occupied before and free after, so it is
       correctly left out. */
    const through: number[] = [];
    for (let j = 0; j < lanes.length; j++) {
      if (j !== lane && before[j] && lanes[j] !== null) through.push(j);
    }

    widest = Math.max(widest, lanes.length);
    rows.push({ lane, above, through, edges });
  }

  return { rows, lanes: widest };
}

function laneX(j: number): number {
  return LANE_PAD + j * LANE_W;
}

const SVG_NS = 'http://www.w3.org/2000/svg';

function svgEl(tag: string, attrs: Record<string, string>): SVGElement {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
}

function refChip(ref: GitRef): HTMLElement {
  const cls = 'git-ref git-ref-' + ref.kind + (ref.head ? ' git-ref-head' : '');
  const what = ref.kind === 'tag' ? 'Tag' : ref.kind === 'remote' ? 'Remote branch' : 'Branch';
  return h('span', { class: cls, title: `${what} ${ref.name}` }, ref.name);
}

/** The scrolling body: graph column + label column, sharing one row height. */
export function graphBody(commits: GitCommit[], workspaceId: string): HTMLElement {
  const { rows, lanes } = layoutGraph(commits);
  const height = rows.length * ROW_H;
  const width = LANE_PAD * 2 + Math.max(lanes, 1) * LANE_W;

  const svg = svgEl('svg', {
    class: 'git-graph-svg',
    width: String(width),
    height: String(height),
    viewBox: `0 0 ${width} ${height}`,
    'aria-hidden': 'true',
  });

  /* group every segment by lane colour so the SVG stays a handful of paths
     rather than one node per segment */
  const segments = new Map<number, string[]>();
  const segment = (lane: number, d: string): void => {
    const list = segments.get(lane);
    if (list) list.push(d);
    else segments.set(lane, [d]);
  };

  rows.forEach((row, i) => {
    const top = i * ROW_H;
    const cy = top + ROW_H / 2;
    const bottom = top + ROW_H;
    const x = laneX(row.lane);

    if (row.above) segment(row.lane, `M${x} ${top}L${x} ${cy}`);
    for (const edge of row.edges) {
      if (edge.from === edge.to) {
        segment(row.lane, `M${x} ${cy}L${x} ${bottom}`);
      } else {
        /* cubic rather than a straight diagonal so a merge reads as a curve */
        const mid = (cy + bottom) / 2;
        const tx = laneX(edge.to);
        segment(row.lane, `M${x} ${cy}C${x} ${mid},${tx} ${mid},${tx} ${bottom}`);
      }
    }
    for (const j of row.through) {
      const jx = laneX(j);
      segment(j, `M${jx} ${top}L${jx} ${bottom}`);
    }
  });

  for (const [lane, ds] of segments) {
    svg.append(svgEl('path', {
      d: ds.join(''),
      fill: 'none',
      stroke: LANE_COLORS[lane % LANE_COLORS.length],
      'stroke-width': '1.5',
    }));
  }
  /* dots last so they sit on top of the lines that meet at them */
  rows.forEach((row, i) => {
    svg.append(svgEl('circle', {
      cx: String(laneX(row.lane)),
      cy: String(i * ROW_H + ROW_H / 2),
      r: String(DOT_R),
      fill: LANE_COLORS[row.lane % LANE_COLORS.length],
    }));
  });

  const labels = h('div', { class: 'git-graph-rows' });
  for (const commit of commits) {
    /* a row opens the commit's detail page as its own tab — setRoute()
       re-activates the existing tab when that commit is already open */
    labels.append(h('button', {
      class: 'git-graph-row',
      type: 'button',
      title: `${commit.short} · ${commit.author} · ${commit.subject || '(no message)'}`,
      dataset: { oid: commit.oid },
      onclick: () => {
        go({ view: 'commit', workspaceId, oid: commit.oid, short: commit.short });
      },
    },
      ...commit.refs.map(refChip),
      h('span', { class: 'git-graph-subject' }, commit.subject || '(no message)'),
      h('span', { class: 'git-graph-meta' }, `${commit.short} · ${rel(commit.timestamp * 1000)}`)));
  }

  return h('div', { class: 'git-graph' }, svg, labels);
}

/* ---------------- self-check ----------------
   Lane assignment is the part that silently produces a wrong-looking graph
   when it breaks, and it is pure, so it gets assertions that run on every dev
   boot. Vite strips the dead branch from production builds.

   It reports, it does not throw. This module is in the boot path
   (main.ts → views/gitPanel → here), so a throw would take the whole renderer
   down and leave the user staring at a blank window — a worse failure than the
   wrong graph it is guarding against. */

function devSelfCheck(): void {
  const commit = (oid: string, parents: string[]): GitCommit => ({
    oid, short: oid.slice(0, 7), parents, author: 't', timestamp: 0, subject: oid, refs: [],
  });

  /*   a ── b ── c     c merges d back in (parents: b, d)
           └── d      d branches from b
     Listed child-first, as --date-order guarantees. */
  const { rows, lanes } = layoutGraph([
    commit('c', ['b', 'd']),
    commit('d', ['b']),
    commit('b', ['a']),
    commit('a', []),
  ]);

  const fail = (msg: string): void => {
    console.error('[gitGraph] lane layout self-check failed: ' + msg);
  };
  if (lanes !== 2) fail('expected 2 lanes, got ' + lanes);
  if (rows[0].lane !== 0 || rows[1].lane !== 1) fail('the merge did not open lane 1 for its second parent');
  /* lane 0 is still expecting b while d is drawn on lane 1, so it passes
     through that row as a plain vertical */
  if (rows[1].through.join() !== '0') fail('lane 0 did not pass through the side branch row');
  if (rows[2].lane !== 0 || !rows[2].above) fail('b did not reuse the lane expecting it');
  /* d freed lane 1 when it consumed it, so the root commit claims the free
     lane 0 rather than reopening lane 1 */
  if (rows[3].lane !== 0 || rows[3].edges.length) fail('the root commit did not close the graph on lane 0');

  for (const row of rows) {
    if (row.through.includes(row.lane)) fail('a commit lane is also drawn as a pass-through');
  }
}

if (import.meta.env.DEV) devSelfCheck();
