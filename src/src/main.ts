/* ============================================================
   Bentomux — calm desktop for AI agent runtime workspaces.
   Entry point: root render, content dispatch, shell wiring, boot.
   ============================================================ */

import '../styles.css';
/* side-effect import: installs `window.bentomux` (the typed IPC bridge)
   before any view module references it. */
import '../preload/bentomux';
import { $, $$, h } from './dom';
import { rel, abs } from './time';
import { ui, type Route, type TabEntry } from './state';
import { db, setDb, branches, runtime, activity } from './store';
import { registerRenderers, render } from './render';
import { renderTabs, activate, stepHistory, registerRestoredTab } from './views/tabs';
import { addWorkspaceFlow, renderSidebar, toggleGitPanel } from './views/sidebar';
import { agentsPage, agentDetailPage } from './views/agents';
import { welcomePage } from './views/welcome';
import { initTerminalEvents, terminalPage, applyTerminalFont } from './views/terminal';
import { initKeyboard } from './keyboard';
import { diffPage } from './views/diff';
import { refreshChangesPill } from './views/gitPanel';
import { initAgentEvents } from './views/agent-events';
import { initAutoUpdate, updateStatus, onUpdateChange } from './updates';
document.documentElement.classList.toggle('macos', /Mac/.test(navigator.platform));

/* Suppress native browser context menu everywhere except inside
   .terminal-page which wires its own contextmenu handler. */
document.addEventListener('contextmenu', e => {
  if (!(e.target as Element).closest('.terminal-page')) e.preventDefault();
});

const MIN_SIDEBAR_WIDTH = 248;
const MAX_SIDEBAR_RATIO = 0.5;

/* ---------------- render root ---------------- */

const PALETTE_CLASSES = ['palette-catppuccin', 'palette-rose-pine', 'palette-gruvbox', 'palette-dracula', 'palette-nord', 'palette-classic', 'palette-eink'];

function applyPaletteClass(palette: string | undefined): void {
  const root = document.documentElement;
  for (const c of PALETTE_CLASSES) root.classList.remove(c);
  if (palette && palette !== 'default') root.classList.add('palette-' + palette);
}

function applyFontPrefs(): void {
  const root = document.documentElement;
  if (db.prefs.font) root.style.setProperty('--term-font', db.prefs.font);
  else root.style.removeProperty('--term-font');
  if (db.prefs.fontSize) root.style.setProperty('--term-font-size', db.prefs.fontSize + 'px');
  else root.style.removeProperty('--term-font-size');
}

function applyShellPrefs(): void {
  document.documentElement.style.setProperty('--sidebar-width', (db.prefs.sidebarWidth || MIN_SIDEBAR_WIDTH) + 'px');
  document.title = 'Bentomux';
  document.body.classList.toggle('pane-hidden', db.prefs.paneHidden === true);
  document.body.classList.toggle('git-panel-open', ui.gitPanelOpen);
  document.documentElement.classList.toggle('dark', resolveTheme() === 'dark');
  applyPaletteClass(db.prefs.palette);
  applyFontPrefs();
  $('#sidebar').classList.toggle('open', ui.sidebarOpen);
  $('#scrim').classList.toggle('show', ui.sidebarOpen);
}

function renderRoot(): void {
  applyShellPrefs();
  renderSidebar();
  renderTabs();
  renderContentInner(ui.route);
}

function findTerminalEntry(route: Route): TabEntry | undefined {
  if (route.view !== 'terminal') return undefined;
  return ui.tabs.find(t => t.route.view === 'terminal' && (t.route === route || t.route.tabId === route.tabId));
}

function renderContentInner(route: Route): void {
  const c = $('#content');
  const body = $('#tabbody');
  c.classList.remove('full');
  c.classList.remove('fullbleed');

  if (route.view === 'terminal') {
    const entry = findTerminalEntry(route);
    const exists = !!entry || db.openTabs.some(t => t.id === route.tabId);
    c.classList.add('full');
    body.innerHTML = '';
    const start = entry && entry.route.view === 'terminal' ? (entry.tree ?? entry.route.tabId) : route.tabId;
    const result = terminalPage(start);
    body.append(exists ? result : h('div', { class: 'page' }, h('p', {}, 'Terminal not found.')));
  } else if (route.view === 'welcome') {
    body.innerHTML = '';
    body.append(welcomePage());
  } else if (route.view === 'agents') {
    ui.route = { view: 'welcome' };
    body.innerHTML = '';
    body.append(welcomePage());
  } else if (route.view === 'agentDetail') {
    body.innerHTML = '';
    body.append(agentDetailPage(route.agentId, route.tab));
  } else if (route.view === 'diff') {
    c.classList.add('fullbleed');
    body.innerHTML = '';
    body.append(diffPage(route.workspaceId, route.path));
  } else {
    body.innerHTML = '';
    body.append(h('div', { class: 'page' }, h('p', {}, 'Unknown view.')));
  }
}

/* ---------------- app bar wiring ---------------- */

function togglePaneHidden(): void {
  db.prefs.paneHidden = !(db.prefs.paneHidden === true);
  void window.bentomux.setPrefs({ paneHidden: db.prefs.paneHidden });
  document.body.classList.toggle('pane-hidden', db.prefs.paneHidden === true);
}

function wirePaneToggle(): void {
  $('#paneBtn').addEventListener('click', togglePaneHidden);
  $('#paneTopBtn').addEventListener('click', togglePaneHidden);
  $('#compactAddBtn').addEventListener('click', () => addWorkspaceFlow());
  $('#compactSettingsBtn').addEventListener('click', () => {
    void import('./views/settings').then(m => m.openSettingsModal());
  });
}

function wireHistory(): void {
  $('#backBtn').addEventListener('click', () => stepHistory(-1));
  $('#redoBtn').addEventListener('click', () => stepHistory(1));
  $('#backTopBtn').addEventListener('click', () => stepHistory(-1));
  $('#redoTopBtn').addEventListener('click', () => stepHistory(1));
}

function wireScrim(): void {
  $('#scrim').addEventListener('pointerdown', () => { ui.sidebarOpen = false; render(); });
}

/* Resolve actual dark/light from pref, falling back to OS for 'system'/undefined */
export function resolveTheme(): 'light' | 'dark' {
  if (db.prefs.theme === 'dark') return 'dark';
  if (db.prefs.theme === 'light') return 'light';
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function toggleTheme(): void {
  db.prefs.theme = db.prefs.theme === 'dark' ? 'light' : 'dark';
  void window.bentomux.setPrefs({ theme: db.prefs.theme });
  document.documentElement.classList.toggle('dark', resolveTheme() === 'dark');
  render(); /* terminals re-theme in place */
}

export function setThemeMode(mode: 'light' | 'dark' | 'system'): void {
  if (db.prefs.theme === mode) return;
  db.prefs.theme = mode;
  void window.bentomux.setPrefs({ theme: mode });
  document.documentElement.classList.toggle('dark', resolveTheme() === 'dark');
  render();
}

export function setPalette(palette: import('../shared/types').PaletteName): void {
  if (db.prefs.palette === palette) return;
  db.prefs.palette = palette;
  void window.bentomux.setPrefs({ palette });
  applyPaletteClass(palette);
  render();
}

export function setTerminalFont(font: string | null | undefined): void {
  db.prefs.font = font === null ? undefined : (font || undefined);
  void window.bentomux.setPrefs({ font: font === null ? null : db.prefs.font });
  applyFontPrefs();
  applyTerminalFont();
}

export function setTerminalFontSize(size: number | undefined): void {
  db.prefs.fontSize = size;
  void window.bentomux.setPrefs({ fontSize: db.prefs.fontSize });
  applyFontPrefs();
  applyTerminalFont();
}

function wireTheme(): void {
  $('#themeBtn').addEventListener('click', toggleTheme);
}

function wireWindowControls(): void {
  window.bentomux.onMaximized(max => {
    ui.maximized = max;
    document.body.classList.toggle('maximized', max);
  });
}

function clampSidebarWidth(clientX: number): number {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(window.innerWidth * MAX_SIDEBAR_RATIO, clientX));
}

function wireSidebarResize(): void {
  const resize = $('#sidebarResize');
  let resizing = false;
  resize.addEventListener('pointerdown', e => {
    if (window.matchMedia('(max-width:900px)').matches) return;
    resizing = true;
    resize.setPointerCapture(e.pointerId);
    document.body.classList.add('resizing-sidebar');
  });
  resize.addEventListener('pointermove', e => {
    if (!resizing) return;
    db.prefs.sidebarWidth = clampSidebarWidth(e.clientX);
    document.documentElement.style.setProperty('--sidebar-width', db.prefs.sidebarWidth + 'px');
  });
  resize.addEventListener('pointerup', () => {
    if (!resizing) return;
    resizing = false;
    void window.bentomux.setPrefs({ sidebarWidth: db.prefs.sidebarWidth });
    document.body.classList.remove('resizing-sidebar');
  });
}

function wireAppBar(): void {
  wirePaneToggle();
  wireHistory();
  wireScrim();
  wireTheme();
  wireWindowControls();
  wireSidebarResize();
  wireGitPill();
}

function wireGitPill(): void {
  const pill = document.getElementById('gitChangesPill');
  if (!pill) return;
  pill.addEventListener('click', () => toggleGitPanel());
}

registerRenderers({ root: renderRoot, content: renderContentInner, sidebar: renderSidebar });

/* ---------------- clock: refresh relative times ---------------- */

/* poll Changes pill every 5 s so +N -N stays accurate as files change,
   even when the git panel is closed */
setInterval(() => { void refreshChangesPill(); }, 5000);

setInterval(() => {
  $$('[data-ts]').forEach(el => {
    const ts = Number((el as HTMLElement).dataset.ts);
    el.textContent = String(ts).length > 6 ? rel(ts) : '';
    el.title = abs(ts);
  });
}, 30000);

/* ---------------- boot ---------------- */

async function loadInitialBranches(): Promise<void> {
  await Promise.all(db.workspaces.map(async w => {
    branches.set(w.id, await window.bentomux.branchFor(w.path));
  }));
}

let statusAudio: AudioContext | null = null;
const lastAgentStates: Record<string, string | null | undefined> = {};

async function playAgentStatusSound(kind: 'finished' | 'blocked' | 'idle'): Promise<void> {
  if (db.prefs.notifEnabled === false || db.prefs.notifSound === false) return;
  const AudioCtor = window.AudioContext ?? (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioCtor) return;
  statusAudio ??= new AudioCtor();
  await statusAudio.resume();
  const now = statusAudio.currentTime;
  const notes = kind === 'blocked' ? [220, 165] : kind === 'finished' ? [660, 880] : [520, 660];
  for (const [index, frequency] of notes.entries()) {
    const oscillator = statusAudio.createOscillator();
    const gain = statusAudio.createGain();
    const at = now + index * 0.12;
    oscillator.frequency.value = frequency;
    gain.gain.setValueAtTime(0.0001, at);
    gain.gain.exponentialRampToValueAtTime(0.22, at + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.22);
    oscillator.connect(gain).connect(statusAudio.destination);
    oscillator.start(at);
    oscillator.stop(at + 0.24);
  }
}

function notifyAgentTransition(id: string, state: string | null | undefined): void {
  const previous = lastAgentStates[id];
  lastAgentStates[id] = state;
  if (previous === undefined || previous === state) return;
  if (state === 'blocked') void playAgentStatusSound('blocked');
  else if (previous === 'working' && state === 'idle') void playAgentStatusSound('finished');
  else if (previous === 'blocked' && state === 'idle') void playAgentStatusSound('idle');
}


function subscribeRuntime(): void {
  window.bentomux.onRuntimeStatus(statuses => {
    for (const [id, st] of Object.entries(statuses)) {
      notifyAgentTransition(id, st.state);
      runtime[id] = st;
      if (st.running) activity[id] = Date.now();
    }
    renderSidebar();
  });
}

function restoreInitialView(restored: import('../shared/types').TabRec[]): void {
  if (restored.length) { activate('term:' + restored[0].id); return; }
  if (!db.workspaces.length) { ui.route = { view: 'welcome' }; }
  render();
}

function logSmokeIfRequested(): void {
  if ((window as unknown as { __BENTOMUX_SMOKE?: boolean }).__BENTOMUX_SMOKE) {
    console.log('[smoke-render] ok workspaces=' + db.workspaces.length + ' tabs=' + ui.tabs.length);
  }
}

function wireUpdateBanner(): void {
  const banner = document.getElementById('updateBanner');
  if (!banner) return;
  const b = banner;
  let dismissed = false;

  function paint(): void {
    const s = updateStatus;
    if (dismissed || (s.phase !== 'available' && s.phase !== 'ready')) {
      b.hidden = true;
      return;
    }
    b.hidden = false;
    b.innerHTML = '';
    const msg = s.phase === 'ready'
      ? 'Bentomux v' + s.available + ' installed — restart to apply'
      : 'Bentomux v' + s.available + ' available';
    const settingsBtn = h('button', {
      class: 'btn primary',
      type: 'button',
      onclick: () => { void import('./views/settings').then(m => m.openSettingsModal()); },
    }, s.phase === 'ready' ? 'Restart' : 'Update');
    const closeBtn = h('button', {
      class: 'btn ghost',
      type: 'button',
      onclick: () => { dismissed = true; b.hidden = true; },
    }, '×');
    b.append(h('span', { class: 'update-banner-msg' }, msg), settingsBtn, closeBtn);
  }

  onUpdateChange(paint);
  paint();
}

async function boot(): Promise<void> {
  const bootStart = performance.now();
  setDb(await window.bentomux.getState());

  /* Sync dark class when OS theme changes and user is on 'system' */
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (!db.prefs.theme || db.prefs.theme === 'system') {
      document.documentElement.classList.toggle('dark', resolveTheme() === 'dark');
      render();
    }
  });

  /* live subscriptions before anything renders */
  initTerminalEvents();
  window.bentomux.onBranch((wsId, branch) => {
    branches.set(wsId, branch);
    renderSidebar();
    renderTabs();
    /* the pill is keyed on the active workspace; if the branch that just
       changed belongs to the active workspace, re-fetch the diff stat so
       the +N -N stays in sync without the user opening the panel. */
    if (wsId === db.activeWorkspaceId) void refreshChangesPill();
  });
  subscribeRuntime();
  initAgentEvents();

  /* restore last session's tabs as fresh shells */
  const restored = await window.bentomux.restoreTabs();
  for (const rec of restored) {
    registerRestoredTab(rec);
    activity[rec.id] = Date.now();
  }

  /* initial branch cache */
  await loadInitialBranches();

  /* initial Changes pill — fetches diff stat for the active workspace so
     the +N -N in the titlebar is accurate before the user opens the panel */
  await refreshChangesPill();

  wireAppBar();
  initKeyboard();

  restoreInitialView(restored);
  logSmokeIfRequested();
  console.info('[perf] renderer-boot-ms=' + Math.round(performance.now() - bootStart));

  /* update check (after render so UI is not blocked) */
  wireUpdateBanner();
  initAutoUpdate(db.prefs.autoUpdate !== false);
}
boot().catch(e => {
  console.error(e);
  document.body.innerText = 'Bentomux boot error: ' + (e instanceof Error ? e.message : String(e));
});
