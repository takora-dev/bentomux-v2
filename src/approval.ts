import { Window } from '@tauri-apps/api/window';
/* ============================================================
   Bentomux — calm desktop for AI agent runtime workspaces.
   Approval overlay page script (approval.html): the always-on-top pill
   shown when a PermissionRequest arrives while Bentomux is not focused.
   Port of the inline <script> from Electron's src/main/overlay.ts.

   The overlay has its OWN webview JS context, so it must import the IPC
   bridge itself, exactly as main.ts does for the main window. It then
   subscribes to the same `agent:approval` and `agent:approvalClosed`
   events the Rust bridge emits, and resolves/denies/jumps via the
   shared bridge methods.
   ============================================================ */

import api from './preload/bentomux';

/* two-tone chime; subject to prefs.notifSound (rendered via CSS class) —
   recreated to mirror Electron's Web Audio implementation */
function playChime(): void {
  try {
    const Ctor = (window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).AudioContext
      || (window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctor) return;
    const ctx = new Ctor();
    const now = ctx.currentTime;
    const note = (freq: number, at: number, dur: number) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = 'sine';
      o.frequency.value = freq;
      g.gain.setValueAtTime(0, at);
      g.gain.linearRampToValueAtTime(.16, at + .015);
      g.gain.exponentialRampToValueAtTime(.0001, at + dur);
      o.connect(g);
      g.connect(ctx.destination);
      o.start(at);
      o.stop(at + dur);
    };
    note(880, now + .02, .16);
    note(1318.5, now + .13, .3);
  } catch {
    /* audio unavailable (autoplay policy) — ignore, overlay still usable */
  }
}

function applyTheme(dark: boolean): void {
  const root = document.documentElement;
  if (dark) {
    root.style.setProperty('--bg', 'rgba(18,18,22,.85)');
    root.style.setProperty('--bg-hover', '#121215');
    root.style.setProperty('--border', 'rgba(255,255,255,.18)');
    root.style.setProperty('--ink', '#fafafa');
    root.style.setProperty('--ink2', '#c8c8ce');
    root.style.setProperty('--sum-bg', 'rgba(255,255,255,.07)');
    root.style.setProperty('--btn-hover', '#3f3f46');
    root.style.setProperty('--shadow', '0 12px 40px rgba(0,0,0,.45)');
  } else {
    root.style.setProperty('--bg', 'rgba(255,255,255,.88)');
    root.style.setProperty('--bg-hover', '#ffffff');
    root.style.setProperty('--border', 'rgba(0,0,0,.14)');
    root.style.setProperty('--ink', '#1f2328');
    root.style.setProperty('--ink2', '#57606a');
    root.style.setProperty('--sum-bg', 'rgba(0,0,0,.055)');
    root.style.setProperty('--btn-hover', 'rgba(0,0,0,.08)');
    root.style.setProperty('--shadow', '0 12px 36px rgba(32,29,25,.22)');
  }
}

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

let currentRequestId: string | null = null;
let currentPaneId: string | null = null;
let currentCwd: string | null = null;

function render(req: {
  requestId: string;
  paneId?: string | null;
  cwd?: string | null;
  toolName?: string;
  summary?: string;
}): void {
  currentRequestId = req.requestId;
  currentPaneId = req.paneId ?? null;
  currentCwd = req.cwd ?? null;
  const where = req.cwd?.split(/[\\/]/).filter(Boolean).pop() || '';
  const tool = req.toolName || 'Tool';
  const summary = req.summary || '';
  $('#where').textContent = where ? ' · ' + where : '';
  $('#tool').textContent = tool;
  $('#sum').textContent = summary;
  playChime();
}

/* resolved elsewhere (native prompt answered, pane died, another surface) */
api.onAgentApprovalClosed(id => {
  if (currentRequestId !== null && id === currentRequestId) {
    void api.hideApproval();
    window.close();
  }
});

/* any request that arrives renders the island; the bridge already filtered
   out the "main window focused & tab on screen" case. */
api.onAgentApproval(r => {
  render(r);
});
/* A new WebView can miss the event emitted during creation. Replay the
   still-pending request so the message and request id are always populated. */
void api.approvalPending().then(req => {
  if (req && currentRequestId === null) render(req);
}).catch(() => { /* overlay remains usable if the bridge is unavailable */ });

/* apply the persisted theme immediately so the island matches the app */
api.getState()
  .then((s: { prefs?: { theme?: string } }) => {
    applyTheme(s.prefs?.theme === 'dark');
    document.documentElement.classList.toggle('dark', s.prefs?.theme === 'dark');
  })
  .catch(() => { /* prefs read failure is non-fatal */ });

$('#approve').addEventListener('click', () => {
  if (!currentRequestId) return;
  void api.resolveApproval(currentRequestId, 'allow')
    .finally(() => { void api.hideApproval(); });
});
$('#deny').addEventListener('click', () => {
  if (!currentRequestId) return;
  void api.resolveApproval(currentRequestId, 'deny')
    .finally(() => { void api.hideApproval(); });
});
$('#jump').addEventListener('click', () => {
  if (!currentRequestId) return;
  void api.hideApproval();
  void api.approvalJump(currentPaneId, currentCwd).catch(() => {});
  void (async () => {
    const main = new Window('main');
    /* let the OS register the window as visible before re-ordering it */
    await new Promise<void>(resolve => window.setTimeout(resolve, 30));
    await main.setVisibleOnAllWorkspaces(true);
    await main.show();
    await main.unminimize();
    await main.setAlwaysOnTop(true);
    await main.setFocus();
    await main.setAlwaysOnTop(false);
  })().catch(() => {});
  window.setTimeout(() => {
    void (async () => {
      const main = new Window('main');
      await main.setVisibleOnAllWorkspaces(true);
      await main.show();
      await main.setFocus();
    })().catch(() => {});
  }, 180);
});