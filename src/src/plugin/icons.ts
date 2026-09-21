/* ---------------- plugin host: shared UI helpers ----------------
   Spec: docs/PLUGIN_PLATFORM.md §7, §11.

   Two jobs, both about the same constraint: `dom.ts` reserves innerHTML for
   compile-time constants and explicitly forbids file content, so a plugin's
   icon is never inline markup. It is either a name from the host's own icon
   set, or an image file served over `plugin://` and rendered as an <img>.

   That sidesteps writing an SVG sanitizer for third-party input — a large,
   easy-to-get-wrong attack surface — at the cost that an image icon does not
   follow the theme's ink colour. */

import { h, markup } from '../dom';
import { IC } from '../icons';
import type { Contribution, PluginIcon } from './types';

/** The host icon a contribution asked for, if it named one we have. */
export function builtinIcon(name: string | undefined): string | null {
  if (!name) return null;
  return (IC as Record<string, string>)[name] ?? null;
}

/**
 * Render a contribution's icon.
 *
 * Returns null when the contribution declared no icon, so callers can fall
 * back to a text-only label rather than rendering an empty box.
 */
export function iconElement(icon: PluginIcon | undefined, assetUrl: (path: string) => string): HTMLElement | null {
  if (!icon) return null;
  if (icon.type === 'builtin') {
    const svg = builtinIcon(icon.name);
    return svg ? markup('span', { class: 'ic' }, svg) : null;
  }
  if (icon.type === 'image' && icon.path) {
    /* an <img> pointing at the plugin's own asset — no markup, no sanitizer */
    return h('img', {
      class: 'plugin-icon',
      src: assetUrl(icon.path),
      alt: '',
      draggable: 'false',
    });
  }
  return null;
}

/** Label for a contribution, falling back to its id's last segment. */
export function contributionLabel(c: Contribution): string {
  if (c.title?.trim()) return c.title;
  const tail = c.id.split('.').pop() ?? c.id;
  return tail;
}

/** `plugin://` URL builder, injected so this module stays free of the loader. */
export type AssetUrlFn = (pluginId: string, path: string) => string;

/** Build the icon for a contribution, given its owning plugin. */
export function contributionIcon(
  pluginId: string,
  c: Contribution,
  assetUrl: AssetUrlFn,
): HTMLElement | null {
  return iconElement(c.icon, path => assetUrl(pluginId, path));
}

/** A tab-strip button for a plugin-contributed tab. */
export function pluginTabIcon(pluginId: string, icon: PluginIcon | undefined, assetUrl: AssetUrlFn): HTMLElement | null {
  return iconElement(icon, path => assetUrl(pluginId, path));
}
