/* ---------------- welcome page (shown when no workspaces exist) ---------------- */

import { h, markup } from '../dom';
import { addWorkspaceFlow } from './sidebar';

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
    ),
  );
}
