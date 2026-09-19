/* ---------------- agent chip ----------------
   One component for every agent mention: cards, modal assignment,
   detected-agents section. Undetected agents render muted with a hint. */

import { h, markup } from '../dom';
import { DIA } from '../icons';
import type { AgentInfo } from '../../shared/types';

export function chipEl(a: AgentInfo | null | undefined, opts: { lg?: boolean; capabilityNote?: string } = {}): HTMLElement {
  if (!a) return h('span', { class: 'chip muted' }, h('span', { class: 'nm' }, 'Unknown'));
  const cls = 'chip' + (opts.lg ? ' lg' : '') + (!a.detected ? ' muted' : '');
  const title = !a.detected ? 'Not detected' : opts.capabilityNote || '';
  return h('span', { class: cls, title: title || null },
    markup('span', { class: 'dia' }, DIA),
    h('span', { class: 'nm' }, a.name));
}
