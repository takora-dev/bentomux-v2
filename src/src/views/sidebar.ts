/* ---------------- sidebar (nav + workspaces + one item per terminal pane) ---------------- */

import { h, $, $$, markup } from '../dom';
import { ic, IC } from '../icons';
import { abs, rel } from '../time';
import { ui, type Route, type TabEntry } from '../state';
import { db, branches, runtime, activity, setDb } from '../store';
import { go } from '../router';
import { render } from '../render';
import { activate, closeTerminalPane, leavesOf, newTerminalTab } from './tabs';
import {
  mostRecentPane,
  primePaneFocus,
  clearTerminalSelections,
} from './terminal';
import { openModal } from '../components/modal';
import { closeContextMenu, contextMenuAnchoredTo, openContextMenu, type MenuEntry } from '../components/menu';
import { openSearchModal } from './search';
import { openSettingsModal } from './settings';
import { remoteDockButton } from './remote';
import { gitPanelPage, refreshChangesPill } from './gitPanel';
import { dockEntries, sidebarEntries } from '../plugin/registry';
import { ensureActive, pluginAssetUrl } from '../plugin/loader';
import { contributionIcon, contributionLabel } from '../plugin/icons';
import api from '../../preload/bentomux';

const PANE_INDENT = 24;

/* Pointer drag state. HTML5 drag/drop is unreliable inside Tauri WebView. */
let dragWsId: string | null = null;
let dragPointerId: number | null = null;
let dragTargetId: string | null = null;
let dragBelow = false;
let dragMoved = false;
let suppressFolderClick = false;
let paneDragId: string | null = null;
let paneDragPointerId: number | null = null;
let paneDragTargetId: string | null = null;
let paneDragMoved = false;
let paneDragBelow = false;
let paneDragStartX = 0;
let paneDragStartY = 0;

interface PaneRow {
  entry: TabEntry;
  paneId: string;
}

interface PaneViewModel {
  entry: TabEntry;
  paneId: string;
  isActive: boolean;
  branch: string | null;
  workspaceName: string;
  status: string;
  agent: string | null;
  timestamp: number;
}

/* every terminal pane living under a workspace — a split tab contributes
   one row per pane, so the list follows the terminal count, not the tab count */
function wsPanes(wsId: string): PaneRow[] {
  const rows: PaneRow[] = [];
  for (const t of ui.tabs.filter(t => t.route.view === 'terminal' && t.workspaceId === wsId)) {
    for (const pid of leavesOf(t)) rows.push({ entry: t, paneId: pid });
  }
  return rows;
}

function paneViewModel({ entry, paneId }: PaneRow): PaneViewModel {
  const st = runtime[paneId];
  const status = st?.state ?? (st?.running ? 'working' : 'idle');
  const agent = st?.runtime || null;
  const ws = db.workspaces.find(w => w.id === entry.workspaceId);
  const isActive = ui.route.view === 'terminal'
    && entry.id === ui.activeTab
    && mostRecentPane(leavesOf(entry)) === paneId;
  return {
    entry,
    paneId,
    isActive,
    branch: branches.get(entry.workspaceId || '') || null,
    workspaceName: ws?.name || 'terminal',
    status,
    agent,
    timestamp: activity[paneId] || Date.now(),
  };
}

function isRevealAnimation(ev: Event): boolean {
  return 'animationName' in ev && (ev as { animationName: string }).animationName === 'workspace-reveal';
}

function onRevealEnd(btn: HTMLElement): void {
  btn.addEventListener('animationend', function once(ev: Event) {
    if (!isRevealAnimation(ev)) return;
    btn.classList.remove('reveal');
    btn.removeEventListener('animationend', once);
  });
}

function paneItem(row: PaneRow, pad: number, reveal: boolean = false): HTMLElement {
  const m = paneViewModel(row);
  const close = h('span', {
    class: 'pane-close',
    role: 'button',
    tabindex: '0',
    title: 'Close terminal',
    'aria-label': 'Close terminal',
  }, '×');
  close.addEventListener('click', e => {
    e.stopPropagation();
    void closeTerminalPane(m.paneId);
  });
  close.addEventListener('pointerdown', e => e.stopPropagation());
  close.addEventListener('keydown', e => {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault();
    e.stopPropagation();
    void closeTerminalPane(m.paneId);
  });
  const btn = h('button', {
    class: 'nav-item sub workspace-child' + (m.isActive ? ' active' : '') + (reveal ? ' reveal' : ''),
    style: 'padding-left:' + pad + 'px',
    'data-pane': m.paneId,
    onclick: () => activatePane(m.paneId, m.entry.id),
  },
    h('span', { class: 'workspace-row' },
      markup('span', { class: 'branch-icon' }, IC.git),
      h('span', { class: 'workspace-copy' },
        h('span', { class: 'workspace-main' },
          h('span', { class: 'workspace-name' }, m.branch || m.workspaceName),
          h('time', { class: 'workspace-time', 'data-ts': String(m.timestamp), title: abs(m.timestamp) }, rel(m.timestamp))),
        h('span', { class: 'workspace-agent' },
          h('span', { class: 'agent-status ' + m.status }, m.status),
          h('span', { class: 'agent-sep' }, '·'),
          h('span', {}, m.agent || 'shell'))),
      close));
  btn.addEventListener('pointerdown', e => startPaneDrag(e, btn, m.paneId));
  btn.addEventListener('pointermove', movePaneDrag);
  btn.addEventListener('pointerup', endPaneDrag);
  btn.addEventListener('pointercancel', finishPaneDrag);
  if (reveal) onRevealEnd(btn);
  return btn;
}

function activatePane(paneId: string, entryId: string): void {
  primePaneFocus(paneId);
  activate(entryId);
}

function clearPaneDropMarks(): void {
  for (const el of $$('.workspace-child.drop-above, .workspace-child.drop-below')) {
    el.classList.remove('drop-above', 'drop-below');
  }
}

function updatePaneDrag(x: number, y: number): void {
  if (!paneDragId) return;
  const target = document.elementFromPoint(x, y)?.closest<HTMLElement>('.workspace-child');
  if (!target || target.dataset.pane === paneDragId) {
    paneDragTargetId = null;
    clearPaneDropMarks();
    return;
  }
  clearPaneDropMarks();
  const bounds = target.getBoundingClientRect();
  paneDragBelow = y > bounds.top + bounds.height / 2;
  target.classList.add(paneDragBelow ? 'drop-below' : 'drop-above');
  paneDragTargetId = target.dataset.pane || null;
}

function finishPaneDrag(): void {
  const sourceId = paneDragId;
  const targetId = paneDragTargetId;
  const below = paneDragBelow;
  paneDragId = null;
  paneDragPointerId = null;
  paneDragTargetId = null;
  paneDragBelow = false;
  clearPaneDropMarks();
  document.body.classList.remove('pane-dragging');
  if (sourceId && targetId && sourceId !== targetId) {
    const moved = ui.tabs.find(t => t.route.view === 'terminal' && leavesOf(t).includes(sourceId));
    if (reorderPane(sourceId, targetId, below) && moved) {
      void api.reorderTabs(
        ui.tabs
          .filter(tab => tab.route.view === 'terminal')
          .map(tab => tab.route.view === 'terminal' ? tab.route.tabId : ''),
      );
      activatePane(sourceId, moved.id);
    }
  }
}

function reorderPane(sourceId: string, targetId: string, below: boolean): boolean {
  const source = ui.tabs.find(t => t.route.view === 'terminal' && leavesOf(t).includes(sourceId));
  const target = ui.tabs.find(t => t.route.view === 'terminal' && leavesOf(t).includes(targetId));
  if (!source || !target || source === target || source.workspaceId !== target.workspaceId) return false;
  const tabs = ui.tabs.filter(t => t.route.view === 'terminal' && t.workspaceId === source.workspaceId);
  const from = tabs.indexOf(source);
  const to = tabs.indexOf(target);
  if (from < 0 || to < 0) return false;
  const [moved] = tabs.splice(from, 1);
  const targetPosition = tabs.indexOf(target);
  tabs.splice(below ? targetPosition + 1 : targetPosition, 0, moved);
  let index = 0;
  ui.tabs = ui.tabs.map(tab => {
    if (tab.route.view !== 'terminal' || tab.workspaceId !== source.workspaceId) return tab;
    return tabs[index++];
  });
  return true;
}

function startPaneDrag(e: PointerEvent, row: HTMLElement, paneId: string): void {
  if (e.button !== 0) return;
  paneDragId = paneId;
  paneDragPointerId = e.pointerId;
  paneDragMoved = false;
  paneDragStartX = e.clientX;
  paneDragStartY = e.clientY;
  row.setPointerCapture(e.pointerId);
}

function movePaneDrag(e: PointerEvent): void {
  if (paneDragPointerId !== e.pointerId || !paneDragId) return;
  if (Math.hypot(e.clientX - paneDragStartX, e.clientY - paneDragStartY) < 5) return;
  paneDragMoved = true;
  document.body.classList.add('pane-dragging');
  updatePaneDrag(e.clientX, e.clientY);
}

function endPaneDrag(e: PointerEvent): void {
  if (paneDragPointerId !== e.pointerId) return;
  if (paneDragMoved) e.preventDefault();
  finishPaneDrag();
}

export function addWorkspaceFlow(): void {
  void api.chooseFolder().then(async path => {
    if (!path) return;
    setDb(await api.addWorkspace(path));
    const added = findAddedWorkspace(path);
    if (!added) return;
    branches.set(added.id, await api.branchFor(added.path));
    db.prefs.expanded = { ...db.prefs.expanded, [added.id]: true };
    void api.setPrefs({ expanded: db.prefs.expanded });
    renderSidebar();
  });
}

function findAddedWorkspace(pickedPath: string) {
  const norm = (s: string): string => s.replace(/[\\/]+$/, '').toLowerCase();
  return db.workspaces.find(w => norm(w.path) === norm(pickedPath));
}

async function performRemoveWorkspace(wsId: string): Promise<void> {
  setDb(await api.removeWorkspace(wsId));
  /* drop local tab entries; main already killed the ptys */
  ui.tabs = ui.tabs.filter(t => !(t.route.view === 'terminal' && t.workspaceId === wsId));
  ui.history = ui.history.filter(id => ui.tabs.some(t => t.id === id));
  ui.future = [];
  ensureSomeActiveTab();
  renderSidebar();
  render();
}

function ensureSomeActiveTab(): void {
  if (ui.tabs.some(t => t.id === ui.activeTab)) return;
  const next = ui.tabs[0];
  if (next) { ui.activeTab = next.id; ui.route = next.route; return; }
  ui.activeTab = null;
  ui.route = { view: 'welcome' };
}

function confirmRemove(ws: { id: string; name: string; path: string }): void {
  const count = wsPanes(ws.id).length;
  const m = openModal({
    title: 'Remove workspace',
    body: h('div', {},
      h('p', { style: 'margin:0 0 6px' }, `Remove “${ws.name}” from Bentomux?`),
      h('p', { style: 'margin:0;color:var(--ink-2)' },
        count ? `${count} open terminal${count > 1 ? 's' : ''} will be closed. The folder itself is not deleted.` : 'The folder itself is not deleted.')),
    footer: h('div', {},
      h('button', { class: 'btn ghost', onclick: () => m.close() }, 'Cancel'),
      h('button', {
        class: 'btn primary',
        onclick: () => { m.close(); void performRemoveWorkspace(ws.id); },
      }, 'Remove')),
  });
}

/* remember each ws's expanded state from the previous render so we can
   detect a fresh expand (false → true) and play the reveal animation only then.
   Initialized lazily on first renderSidebar() because `db` is still undefined
   when this module is first imported (set later via setDb() from main). */
let lastExpanded: Record<string, boolean> | null = null;

function navItem(label: string, iconName: keyof typeof IC, view: Route['view']): HTMLElement {
  const active = ui.route.view === view
    || (view === 'agents' && ui.route.view === 'agentDetail');
  return h('button', {
    class: 'nav-item' + (active ? ' active' : ''),
    /* widen to Route so TypeScript doesn't demand the per-view discriminator
       fields (agentId, tabId) we don't use for the simple nav-row case */
    onclick: () => go({ view } as Route),
  }, ic(iconName), h('span', {}, label));
}

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
const SEARCH_SHORTCUT = isMac ? '\u2318K' : 'Ctrl K';

function searchTriggerButton(): HTMLElement {
  return h('button', {
    class: 'nav-item search-trigger',
    type: 'button',
    title: 'Search menus, panes, and tabs (' + (isMac ? '\u2318K' : 'Ctrl+K') + ')',
    'aria-label': 'Open search',
    onclick: () => openSearchModal(),
  },
    ic('search'),
    h('span', {}, 'Search'),
    h('span', { class: 'shortcut' }, SEARCH_SHORTCUT));
}


function settingsGearButton(): HTMLElement {
  return h('button', {
    class: 'iconbtn sidebar-gear',
    type: 'button',
    onclick: () => openSettingsModal(),
    title: 'Open settings',
    'aria-label': 'Open settings',
  }, ic('gear'));
}

/* ---------------- plugin-contributed sidebar rows ----------------
   A plugin row fires its declared command. The command's handler may live in
   a plugin that has not been imported yet — plugin activation is lazy — so
   the click activates first and runs after. */

function pluginSidebarRows(): HTMLElement[] {
  return sidebarEntries().map(entry => {
    const icon = contributionIcon(entry.pluginId, entry.contribution, pluginAssetUrl);
    const label = contributionLabel(entry.contribution);
    return h('button', {
      class: 'nav-item plugin-nav-item',
      type: 'button',
      title: `${label} — ${entry.pluginName}`,
      dataset: { plugin: entry.pluginId, contribution: entry.contribution.id },
      onclick: () => {
        void ensureActive(entry.pluginId).then(ok => {
          if (ok) entry.run();
          else console.warn(`[plugin:${entry.pluginId}] could not activate to run \`${entry.contribution.id}\``);
        });
      },
    }, icon ?? ic('board'), h('span', {}, label));
  });
}

function pluginDockButtons(): HTMLElement[] {
  return dockEntries().map(entry => {
    const icon = contributionIcon(entry.pluginId, entry.contribution, pluginAssetUrl);
    const label = contributionLabel(entry.contribution);
    return h('button', {
      class: 'iconbtn plugin-dock-item',
      type: 'button',
      title: `${label} — ${entry.pluginName}`,
      'aria-label': label,
      dataset: { plugin: entry.pluginId, contribution: entry.contribution.id },
      onclick: () => {
        void ensureActive(entry.pluginId).then(ok => {
          if (ok) entry.run();
        });
      },
    }, icon ?? ic('board'));
  });
}

export function toggleGitPanel(): void {
  clearTerminalSelections();
  ui.gitPanelOpen = !ui.gitPanelOpen;
  document.body.classList.toggle('git-panel-open', ui.gitPanelOpen);
  renderSidebar();
}

function workspaceLabelRow(): HTMLElement {
  return h('div', { class: 'ws-label' },
    h('span', { class: 'nav-label' }, 'Workspace'),
    h('button', { class: 'ws-add', title: 'Add workspace folder', 'aria-label': 'Add workspace folder', onclick: () => addWorkspaceFlow() }, ic('plus')));
}

function toggleWorkspaceExpanded(wsId: string, currentlyOpen: boolean): void {
  db.prefs.expanded = { ...(db.prefs.expanded || {}), [wsId]: !currentlyOpen };
  void api.setPrefs({ expanded: db.prefs.expanded });
  db.activeWorkspaceId = wsId;
  void api.setActiveWorkspace(wsId);
  /* clicking a workspace folder also marks it active; re-fetch the titlebar
     Changes pill so +N -N matches the newly-active folder */
  void refreshChangesPill();
  renderSidebar();
}

/* ---------------- pointer drag & drop reordering of workspace folders ---------------- */

function clearRowDropMark(row: HTMLElement): void {
  row.classList.remove('drop-above', 'drop-below');
}

function clearDropMarks(): void {
  for (const el of $$('.folder.drop-above, .folder.drop-below')) {
    clearRowDropMark(el);
  }
}

function markDropTarget(row: HTMLElement, below: boolean): void {
  clearDropMarks();
  row.classList.add(below ? 'drop-below' : 'drop-above');
  dragTargetId = row.dataset.ws || null;
  dragBelow = below;
}

function updateFolderDrag(x: number, y: number): void {
  if (!dragWsId) return;
  const target = document.elementFromPoint(x, y)?.closest('.folder');
  if (!(target instanceof HTMLElement) || target.dataset.ws === dragWsId) {
    dragTargetId = null;
    clearDropMarks();
    return;
  }
  const bounds = target.getBoundingClientRect();
  markDropTarget(target, y > bounds.top + bounds.height / 2);
}

function finishFolderDrag(): void {
  const sourceId = dragWsId;
  const targetId = dragTargetId;
  const below = dragBelow;
  dragWsId = null;
  dragPointerId = null;
  dragTargetId = null;
  clearDropMarks();
  document.body.classList.remove('workspace-dragging');
  if (sourceId && targetId && sourceId !== targetId) {
    void commitWorkspaceReorder(sourceId, targetId, below);
  }
}

function startFolderPointerDrag(e: PointerEvent, row: HTMLElement, wsId: string): void {
  if (e.button !== 0 || (e.target instanceof HTMLElement &&
      e.target.closest('.ws-menu'))) return;
  dragWsId = wsId;
  dragPointerId = e.pointerId;
  dragMoved = false;
  row.setPointerCapture(e.pointerId);
  e.preventDefault();
}

function moveFolderPointerDrag(e: PointerEvent): void {
  if (dragPointerId !== e.pointerId || !dragWsId) return;
  dragMoved = true;
  document.body.classList.add('workspace-dragging');
  updateFolderDrag(e.clientX, e.clientY);
}

function endFolderPointerDrag(e: PointerEvent): void {
  if (dragPointerId !== e.pointerId) return;
  if (dragMoved) {
    e.preventDefault();
    suppressFolderClick = true;
  }
  finishFolderDrag();
}

/* persist the dragged workspace above/below the drop target */
async function commitWorkspaceReorder(
  dragId: string,
  targetWsId: string,
  below: boolean,
): Promise<void> {
  const ids = db.workspaces.map(w => w.id);
  const from = ids.indexOf(dragId);
  if (from < 0 || !ids.includes(targetWsId)) return;
  ids.splice(from, 1);
  const to = ids.indexOf(targetWsId);
  ids.splice(below ? to + 1 : to, 0, dragId);
  setDb(await api.reorderWorkspaces(ids));
  renderSidebar();
}

function workspaceFolder(ws: { id: string; name: string; path: string }, open: boolean): HTMLElement {
  const row = h('button', {
    class: 'folder' + (open ? ' open' : ''),
    title: ws.path,
    'data-ws': ws.id,
    onclick: () => {
      if (suppressFolderClick) {
        suppressFolderClick = false;
        return;
      }
      toggleWorkspaceExpanded(ws.id, open);
    },
  },
    markup('span', { class: 'folder-icon' }, IC.folder),
    h('span', { class: 'folder-name' }, ws.name),
    workspaceMenuSpan(ws));
  row.addEventListener('pointerdown', e => startFolderPointerDrag(e, row, ws.id));
  row.addEventListener('pointermove', moveFolderPointerDrag);
  row.addEventListener('pointerup', endFolderPointerDrag);
  row.addEventListener('pointercancel', finishFolderDrag);
  return row;
}

/* ⋯ toggle that opens the per-workspace action menu (new terminal / remove) */
function workspaceMenuSpan(ws: { id: string; name: string; path: string }): HTMLElement {
  const span = h('span', {
    class: 'ws-menu',
    title: 'Workspace actions',
    'aria-label': 'Workspace actions',
    role: 'button',
  }, markup('span', { class: 'ws-menu-icon' }, IC.dots));
  span.addEventListener('click', (e: Event) => {
    e.stopPropagation(); /* opening the menu must not also expand/collapse the folder */
    if (contextMenuAnchoredTo(span)) { closeContextMenu(); return; }
    const r = span.getBoundingClientRect();
    const entries: MenuEntry[] = [
      { label: 'New terminal', action: () => void newTerminalTab(ws.id) },
      { sep: true },
      { label: 'Remove workspace', action: () => confirmRemove(ws) },
    ];
    openContextMenu(r.left, r.bottom + 4, entries, span);
  });
  return span;
}

function renderWorkspaces(): void {
  const newExpanded: Record<string, boolean> = {};
  for (const ws of db.workspaces) {
    const open = db.prefs.expanded?.[ws.id] !== false;
    /* reveal only on a true user-driven expand (false → true). A ws we haven't
       seen before is treated as "unknown", not "was-open", so first render and
       brand-new workspaces never animate (avoids the run-once startup flash). */
    const prev = lastExpanded![ws.id];
    const reveal = open && prev === false;
    newExpanded[ws.id] = open;
    const nav = $('#nav');
    nav.append(workspaceFolder(ws, open));
    if (!open) continue;
    const kids = wsPanes(ws.id);
    for (const row of kids) nav.append(paneItem(row, PANE_INDENT, reveal));
    if (!kids.length) nav.append(h('div', { class: 'empty-note' }, 'No terminals'));
  }
  lastExpanded = newExpanded;
}

export function renderSidebar(): void {
  if (lastExpanded == null) lastExpanded = { ...(db.prefs?.expanded || {}) };
  const nav = $('#nav');
  nav.innerHTML = '';

  nav.append(searchTriggerButton(), workspaceLabelRow());

  renderWorkspaces();

  if (!db.workspaces.length) nav.append(h('div', { class: 'empty-note' }, 'Add a folder to begin'));

  /* plugin rows sit after the workspace list and before the dock: they are
     the user's own additions, so they read as a section of their own */
  const pluginRows = pluginSidebarRows();
  if (pluginRows.length) {
    nav.append(h('div', { class: 'nav-label plugin-nav-label' }, 'Plugins'), ...pluginRows);
  }

  renderBottomDock();
  renderGitPanel();
}

/* bottom dock sits below the scrollable nav list and is always visible —
   the settings gear and the remote toggle live here so they don't get
   pushed out of reach by long workspace lists. */
function renderBottomDock(): void {
  const dock = $('#sidebarBottom');
  if (!dock) return;
  dock.innerHTML = '';
  dock.append(...pluginDockButtons(), settingsGearButton(), remoteDockButton());
}

function renderGitPanel(): void {
  /* mirror body class so the .win grid 4th column animates correctly even
     when applyShellPrefs() has not run yet (first paint after a render()). */
  document.body.classList.toggle('git-panel-open', ui.gitPanelOpen);
  const slot = $('#gitPanelBody');
  if (!slot) return;
  if (!ui.gitPanelOpen) {
    /* drop the mounted panel so re-opening fetches instead of showing the
       snapshot from the previous time the panel was up */
    slot.innerHTML = '';
    return;
  }
  /* gitPanelPage() returns the same node while the active workspace is
     unchanged, so the 1 Hz runtime-status re-render does not rebuild the
     panel (and re-run its git shellouts) underneath the user. */
  const panel = gitPanelPage();
  if (slot.firstElementChild !== panel) {
    slot.innerHTML = '';
    slot.append(panel);
  }
}
