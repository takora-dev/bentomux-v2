/* Peek at what a pane is actually showing, over the remote socket: send
   something, then print the screen the desktop renders for it. Diagnostic
   for the marker trick in probe-remote-session.mjs.
   Run: node scripts/dump-pane-view.mjs <paneId> */
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const paneId = process.argv[2];
const keys = process.argv.slice(3);
if (!paneId) {
  console.error('usage: node scripts/dump-pane-view.mjs <paneId> [keys to type…]');
  process.exit(1);
}
const state = JSON.parse(
  readFileSync(join(homedir(), 'Library/Application Support/app.bentomux.dev/bentomux.json'), 'utf8'),
);
const port = state.prefs.remote.port ?? 8765;
const res = await fetch(`http://127.0.0.1:${port}/?t=${encodeURIComponent(state.prefs.remote.token)}`, {
  redirect: 'manual',
});
const cookie = (res.headers.getSetCookie?.() ?? []).map((c) => c.split(';')[0]).join('; ');

const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`, { headers: { cookie } });
const seen = [];
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = async (pred, ms = 20000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const hit = seen.find(pred);
    if (hit) return hit;
    await wait(60);
  }
  return null;
};
ws.onmessage = (e) => seen.push(JSON.parse(e.data));
ws.onopen = async () => {
  await until((m) => m.t === 'panes');
  ws.send(JSON.stringify({ t: 'watch', paneId }));
  for (const k of keys) {
    await wait(400);
    ws.send(JSON.stringify({ t: 'write', paneId, data: k.endsWith('\n') ? `${k}\n` : k }));
  }
  const view = await until((m) => m.t === 'view' && m.paneId === paneId, 6000);
  await wait(1500);
  const last = seen.filter((m) => m.t === 'view' && m.paneId === paneId).pop() ?? view;
  console.log(last?.text ?? '(no view)');
  const status = seen.filter((m) => m.t === 'status').pop();
  console.log('--- runtime:', JSON.stringify(status?.statuses?.[paneId] ?? null));
  ws.close();
};
