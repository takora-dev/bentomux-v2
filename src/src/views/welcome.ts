/* ---------------- welcome page (shown when no workspaces exist) ---------------- */

import { h, markup } from '../dom';
import { addWorkspaceFlow } from './sidebar';
import { widgetEntries, widgetRenderer } from '../plugin/registry';
import { ensureActive, pluginAssetUrl } from '../plugin/loader';
import { contributionIcon, contributionLabel } from '../plugin/icons';

const LOGO_SVG = `<svg width="48" height="48" viewBox="0 0 48 48" fill="none" xmlns="http://www.w3.org/2000/svg">
  <rect width="48" height="48" rx="10" fill="currentColor" opacity="0.08"/>
  <rect x="8" y="8" width="14" height="14" rx="2" fill="currentColor" opacity="0.7"/>
  <rect x="26" y="8" width="14" height="14" rx="2" fill="currentColor"/>
  <rect x="8" y="26" width="14" height="14" rx="2" fill="currentColor"/>
  <rect x="26" y="26" width="14" height="14" rx="2" fill="currentColor" opacity="0.5"/>
</svg>`;

const STEPS: { num: string; title: string; desc: string }[] = [
  { num: '1', title: 'Add a workspace folder', desc: 'Click "Add workspace" or use the + button in the sidebar to pick a project folder.' },
  { num: '2', title: 'Open a terminal', desc: 'Click your workspace in the sidebar to open a terminal pane inside that folder.' },
  { num: '3', title: 'Run an AI agent', desc: 'Start Claude Code, Pi, or any configured agent from within your terminal.' },
];

export function welcomePage(): HTMLElement {
  const steps = STEPS.map(s =>
    h('div', { class: 'welcome-step' },
      h('div', { class: 'welcome-step-num' }, s.num),
      h('div', { class: 'welcome-step-body' },
        h('strong', {}, s.title),
        h('p', {}, s.desc),
      ),
    ),
  );

  return h('div', { class: 'page welcome-page' },
    h('div', { class: 'welcome-inner' },
      markup('div', { class: 'welcome-logo' }, LOGO_SVG),
      h('h1', { class: 'welcome-title' }, 'Bentomux'),
      h('p', { class: 'welcome-sub' }, 'Calm desktop for AI agent workspaces'),
      h('div', { class: 'welcome-steps' }, steps),
      h('button', {
        class: 'btn primary welcome-cta',
        onclick: () => addWorkspaceFlow(),
      }, '+ Add workspace'),
      widgetSection(),
    ),
  );
}

/* ---------------- plugin-contributed widgets ----------------
   Widgets live here because the welcome page is the app's dashboard — the
   surface you see when nothing is open. Each card owns its body: the plugin
   renders into the host element it is given, and any cleanup it returns runs
   when the page is torn down. */

function widgetSection(): HTMLElement | null {
  const entries = widgetEntries();
  if (!entries.length) return null;

  const cards = entries.map(entry => {
    const body = h('div', { class: 'widget-body' });
    const title = contributionLabel(entry.contribution);
    const card = h('div', {
      class: 'widget-card',
      dataset: { plugin: entry.pluginId, contribution: entry.contribution.id },
    },
      h('div', { class: 'widget-head' },
        contributionIcon(entry.pluginId, entry.contribution, pluginAssetUrl) ?? h('span'),
        h('span', { class: 'widget-title' }, title),
        h('span', { class: 'widget-source', title: entry.pluginName }, entry.pluginName),
      ),
      body,
    );

    /* a widget from a lazily-activated plugin has no renderer yet: activate,
       then render, so the card fills in rather than staying blank */
    const paint = (): void => {
      const render = widgetRenderer(entry.pluginId, entry.contribution.id);
      if (!render) {
        body.textContent = '';
        return;
      }
      body.textContent = '';
      try {
        const cleanup = render(body);
        if (typeof cleanup === 'function') widgetCleanups.push(cleanup);
      } catch (e) {
        console.error(`[plugin:${entry.pluginId}] widget \`${entry.contribution.id}\` threw`, e);
        body.textContent = 'This widget failed to render.';
      }
    };

    void ensureActive(entry.pluginId).then(() => paint());
    return card;
  });

  return h('div', { class: 'welcome-widgets' },
    h('div', { class: 'nav-label' }, 'Widgets'),
    h('div', { class: 'widget-grid' }, cards),
  );
}

/* cleanups from the previous welcome-page mount; run before the next one so
   a re-render does not leave plugin listeners behind */
const widgetCleanups: Array<() => void> = [];

export function disposeWidgets(): void {
  for (const cleanup of widgetCleanups.splice(0)) {
    try {
      cleanup();
    } catch (e) {
      console.error('[plugin-host] widget cleanup threw', e);
    }
  }
}
