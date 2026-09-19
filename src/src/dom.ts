/* ---------------- tiny DOM helper ---------------- */

export const $ = (s: string, r: ParentNode = document): HTMLElement => r.querySelector(s) as HTMLElement;
export const $$ = (s: string, r: ParentNode = document): HTMLElement[] => [...r.querySelectorAll(s)] as HTMLElement[];

export type Kid = Node | string | null | false | undefined | Kid[];
export type Attrs = Record<string, unknown>;

/* Only place in the renderer allowed to set innerHTML. `trusted` must be a
   compile-time constant from this bundle (icon SVG); never user, agent, or
   file content. Everything else goes through h() text children. */
export function markup(tag: string, attrs: Attrs | null, trusted: string): HTMLElement {
  const el = h(tag, attrs);
  el.innerHTML = trusted;
  return el;
}

export function h(tag: string, attrs?: Attrs | null, ...kids: Kid[]): HTMLElement {
  const el = document.createElement(tag);
  for (const [k, v0] of Object.entries(attrs || {})) {
    if (v0 == null || v0 === false) continue;
    const v = v0 as unknown;
    if (k === 'class') el.className = String(v);
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v as EventListener);
    else el.setAttribute(k, v === true ? '' : String(v));
  }
  for (const kid of kids.flat(9)) {
    if (kid == null || kid === false) continue;
    el.append(kid as unknown as Node);
  }
  return el;
}
