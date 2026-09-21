/* ---------------- Git Tools (right-side panel) ----------------
   Shows status, a navigable file list (clicking a file opens the diff in
   its own titlebar tab). Also owns the live `+N -N` text for the Changes
   pill in the titlebar; `main.ts` imports `refreshChangesPill` to trigger
   the update at boot (and any time the active workspace changes) so the
   pill is accurate even before the user opens the panel. */

import { h } from '../dom';
import { db } from '../store';
import { ui } from '../state';
import { go } from '../router';
import type {
  GitStatusResult, GitRemoteInfo, GitStatusEntry,
} from '../../shared/types';
import api from '../../preload/bentomux';

interface PanelState {
  status: GitStatusResult | null;
  remote: GitRemoteInfo | null;
  /* a refresh (mount or button) is in flight and the panel shows a spinner */
  loading: boolean;
  /* any git pass is in flight, including silent background polls */
  inflight: boolean;
}

/* the panel is mounted once per (open, workspace) pair and then polls itself,
   so the sidebar's 1 Hz re-render no longer re-runs the git shellouts */
const PANEL_POLL_MS = 4000;

function activeWsId(): string | null {
  return db.activeWorkspaceId;
}

function formatStatusBadge(xy: string): string {
  /* a one-letter per side (X = index, Y = worktree) is enough for the user;
     the full XY is in the title attribute. */
  if (xy === '??') return 'U';
  if (xy === '!!') return 'I';
  const x = xy[0] || ' ';
  const y = xy[1] || ' ';
  return (x === ' ' ? '·' : x) + (y === ' ' ? '·' : y);
}

function fileRowTitle(xy: string): string {
  if (xy === '??') return 'Untracked';
  if (xy === '!!') return 'Ignored';
  const map: Record<string, string> = {
    M: 'Modified', A: 'Added', D: 'Deleted', R: 'Renamed', C: 'Copied', T: 'Type changed',
  };
  const x = xy[0] || ' ';
  const y = xy[1] || ' ';
  return [map[x] || x, map[y] || y].filter(s => s !== ' ').join(' / ');
}

function buildHeader(state: PanelState, paint: () => void): HTMLElement {
  const ws = db.workspaces.find(w => w.id === activeWsId());
  const name = ws?.name || 'No workspace';
  const branch = state.status?.branch ?? null;
  return h('div', { class: 'right-sidebar-h' },
    h('div', { class: 'grow' },
      h('b', {}, 'Git · ' + name),
      h('div', { class: 'muted', style: 'margin-top:2px' }, state.status?.isRepo === false ? 'Not a git repository' : (branch ? 'On ' + branch : 'Detached HEAD'))),
    h('button', {
      class: 'tbtn',
      title: state.loading ? 'Refreshing…' : 'Refresh',
      onclick: () => void refresh(state, paint),
      'aria-label': 'Refresh',
      disabled: state.loading,
    },
      h('span', { style: 'font-size:14px' }, '↻')));
}

function buildFilesList(state: PanelState): HTMLElement {
  const list = h('div', { class: 'git-files-list' });
  const wrap = h('div', { class: 'git-section git-files' },
    h('div', { class: 'git-section-h' },
      h('span', { class: 'grow' }, 'Changes'),
      h('span', { class: 'muted' }, state.loading
        ? 'Refreshing…'
        : (state.status?.entries.length ?? 0) + ' files')),
    list);

  if (!state.status) {
    list.append(h('div', { class: 'git-empty' }, 'Loading…'));
    return wrap;
  }
  if (state.status.isRepo === false) {
    list.append(h('div', { class: 'git-empty' }, 'Initialize a repository to use Git Tools.'));
    return wrap;
  }
  if (!state.status.entries.length) {
    list.append(h('div', { class: 'git-empty' }, 'Working tree clean.'));
    return wrap;
  }
  const wsId = activeWsId();
  if (!wsId) return wrap;
  for (const entry of state.status.entries) list.append(fileRow(entry, wsId));
  return wrap;
}

function fileRow(entry: GitStatusEntry, wsId: string): HTMLElement {
  /* the right panel's file list is a navigator; each file's diff opens as
     its own titlebar tab (route `{ view: 'diff', workspaceId, path }` —
     setRoute() re-activates the existing tab when the file is already open).
     The active file is whichever one `ui.route` is currently viewing. */
  const active = ui.route.view === 'diff' && ui.route.workspaceId === wsId && ui.route.path === entry.path;
  const btn = h('button', {
    class: 'git-file-row' + (active ? ' active' : ''),
    type: 'button',
    title: fileRowTitle(entry.status) + ' — ' + entry.path,
    onclick: () => { go({ view: 'diff', workspaceId: wsId, path: entry.path }); },
  },
    h('span', { class: 'xy' }, formatStatusBadge(entry.status)),
    h('span', { class: 'name' }, entry.path));
  return btn;
}

/* ---------------- data flow ---------------- */

interface RefreshOpts {
  /* background poll: no spinner, and no repaint when the data did not move,
     so the file list keeps its scroll position while the panel sits idle */
  silent?: boolean;
}

async function refresh(state: PanelState, paint: () => void, opts: RefreshOpts = {}): Promise<void> {
  if (state.inflight) return; /* one git pass at a time per panel */
  state.inflight = true;
  const before = JSON.stringify([state.status, state.remote]);
  if (!opts.silent) {
    state.loading = true;
    paint();
  }
  try {
    const wsId = activeWsId();
    if (!wsId) {
      state.status = { isRepo: false, branch: null, ahead: 0, behind: 0, entries: [], hasUntracked: false };
      state.remote = { isRepo: false, remote: null, defaultBranch: null };
    } else {
      const [s, r] = await Promise.all([api.gitStatus(wsId), api.gitRemoteInfo(wsId)]);
      state.status = s;
      state.remote = r;
    }
    void refreshChangesPill();
  } catch (e: unknown) {
    console.error('gitPanel refresh failed', e);
  } finally {
    state.inflight = false;
    state.loading = false;
  }
  if (!opts.silent || JSON.stringify([state.status, state.remote]) !== before) paint();
}

/* ---------------- Changes pill ----------------
   The pill lives in the titlebar (outside this panel's DOM). It needs to
   be accurate even before the panel is opened, so the boot flow in
   main.ts calls `refreshChangesPill()` directly. This module owns the
   DOM mutation and the "no workspace / not a git repo → hide" rule. */

function paintChangesPill(isRepo: boolean, additions: number, deletions: number): void {
  const pill = document.getElementById('gitChangesPill');
  if (!pill) return;
  if (!isRepo) { pill.setAttribute('hidden', ''); return; }
  pill.removeAttribute('hidden');
  const add = pill.querySelector('[data-role="add"]');
  const del = pill.querySelector('[data-role="del"]');
  if (add) add.textContent = '+' + additions;
  if (del) del.textContent = '-' + deletions;
}

export async function refreshChangesPill(): Promise<void> {
  const wsId = activeWsId();
  if (!wsId) { paintChangesPill(false, 0, 0); return; }
  try {
    const ds = await api.gitDiffStat(wsId);
    paintChangesPill(ds.isRepo, ds.additions, ds.deletions);
  } catch (e: unknown) {
    console.error('refreshChangesPill failed', e);
    paintChangesPill(false, 0, 0);
  }
}

/* ---------------- mount ---------------- */

/* the node the sidebar currently has mounted; `isConnected` is the invalidation
   signal — the sidebar clears the slot when the panel is closed, so a detached
   root means "rebuild (and re-fetch) on the next open" */
let mounted: { wsId: string | null; root: HTMLElement } | null = null;

export function gitPanelPage(): HTMLElement {
  const wsId = activeWsId();
  if (mounted && mounted.wsId === wsId && mounted.root.isConnected) return mounted.root;

  const root = h('div', { class: 'git-panel-root' });

  const state: PanelState = {
    status: null, remote: null, loading: false, inflight: false,
  };

  function paint(): void {
    root.innerHTML = '';
    root.append(
      buildHeader(state, paint),
      h('div', { class: 'git-panel-body' },
        buildFilesList(state)),
    );
  }

  mounted = { wsId, root };
  paint();
  void refresh(state, paint);

  /* the panel owns its own freshness now that it is not re-mounted: slow poll
     while it is on screen, self-clearing once the sidebar drops the node */
  const poll = window.setInterval(() => {
    if (!root.isConnected) { window.clearInterval(poll); return; }
    void refresh(state, paint, { silent: true });
  }, PANEL_POLL_MS);

  return root;
}
