/* ---------------- Shiki singleton (lazy, fine-grained) ----------------
   One HighlighterCore for the whole renderer. Languages are dynamic-imported
   the first time they're needed; the highlighter itself is built with the
   JavaScript regex engine (no WASM) so first paint isn't blocked on the
   Oniguruma wasm fetch.

   We use a SINGLE theme (`github-dark`) only so Shiki has something to
   tokenize against — we never read the colors. The output that reaches the
   DOM is TextMate scopes -> our own `TokenKind` (see scopeMap.ts), and the
   per-palette colors come from `styles.css` via `.syn-*` rules. */

import type { HighlighterCore, LanguageInput } from 'shiki/core';
import type { LangId } from './lang';

/* Shiki's `lang` field accepts any bundled-language id (a string union),
   but the type isn't exported. We model it as `string` internally and cast
   at the API boundary. The set of valid ids is exactly what's in
   LANG_LOADERS below. */
type ShikiLang = string;

/* Map our short LangId -> Shiki's bundled-language id. The renderer-side
   EXT table gives us `ts`/`py`/`go`/etc; Shiki uses the full name. */
const SHIKI_LANG: Record<LangId, ShikiLang> = {
  ts: 'typescript',
  tsx: 'tsx',
  js: 'javascript',
  jsx: 'jsx',
  mjs: 'javascript',
  cjs: 'javascript',
  py: 'python',
  rb: 'ruby',
  go: 'go',
  rs: 'rust',
  java: 'java',
  kt: 'kotlin',
  cs: 'csharp',
  cpp: 'cpp',
  c: 'c',
  h: 'c',
  swift: 'swift',
  php: 'php',
  sh: 'bash',
  sql: 'sql',
  html: 'html',
  css: 'css',
  scss: 'scss',
  json: 'json',
  yaml: 'yaml',
  toml: 'toml',
  md: 'markdown',
  xml: 'xml',
  lua: 'lua',
  pl: 'perl',
  r: 'r',
  dart: 'dart',
};

let highlighterPromise: Promise<HighlighterCore> | null = null;
const languagePromises = new Map<ShikiLang, Promise<void>>();

/* Keep common grammars bundled as lazy chunks. Less common languages remain
   detectable and fall back to plain text when no loader is registered.
   NOTE: only the JS/TS family + python/markdown/json stay eagerly reachable
   as separate chunks — diffs of those cover the vast majority of views.
   The rest (go/rust/bash/sql/html/css/yaml/toml/xml) load on first use via
   the same dynamic import, so adding them back costs one chunk fetch, not
   a bigger initial bundle. */
const LANG_LOADERS: Partial<Record<ShikiLang, () => Promise<{ default: LanguageInput }>>> = {
  javascript: () => import('@shikijs/langs/javascript'),
  typescript: () => import('@shikijs/langs/typescript'),
  tsx: () => import('@shikijs/langs/tsx'),
  jsx: () => import('@shikijs/langs/jsx'),
  python: () => import('@shikijs/langs/python'),
  go: () => import('@shikijs/langs/go'),
  rust: () => import('@shikijs/langs/rust'),
  bash: () => import('@shikijs/langs/bash'),
  sql: () => import('@shikijs/langs/sql'),
  html: () => import('@shikijs/langs/html'),
  css: () => import('@shikijs/langs/css'),
  json: () => import('@shikijs/langs/json'),
  yaml: () => import('@shikijs/langs/yaml'),
  toml: () => import('@shikijs/langs/toml'),
  markdown: () => import('@shikijs/langs/markdown'),
  xml: () => import('@shikijs/langs/xml'),
};


async function getHighlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    highlighterPromise = (async () => {
      const { createHighlighterCore } = await import('shiki/core');
      const { createJavaScriptRegexEngine } = await import('@shikijs/engine-javascript');
      return createHighlighterCore({
        /* Tokenization only needs scope explanations; CSS owns all colors. */
        themes: [{ name: 'bentomux', settings: [] }],
        langs: [],
        engine: createJavaScriptRegexEngine({ forgiving: true }),
      });
    })();
  }
  return highlighterPromise;
}

/* resolve a renderer-side LangId to the Shiki language id (or null if the
   id is somehow unknown to our map). */
export function shikiIdFor(lang: LangId): ShikiLang | null {
  return SHIKI_LANG[lang] ?? null;
}

/* ensure a language is loaded into the highlighter, dynamic-importing its
   grammar on first use. Idempotent: if already loaded, no-op. */
export async function ensureLang(lang: LangId): Promise<ShikiLang | null> {
  const hl = await getHighlighter();
  const shikiId = SHIKI_LANG[lang];
  if (!shikiId) return null;
  if (hl.getLoadedLanguages().includes(shikiId)) return shikiId;
  const loader = LANG_LOADERS[shikiId];
  if (!loader) return null;

  let loading = languagePromises.get(shikiId);
  if (!loading) {
    loading = (async () => {
      const mod = await loader();
      if (!hl.getLoadedLanguages().includes(shikiId)) await hl.loadLanguage(mod.default);
    })();
    languagePromises.set(shikiId, loading);
  }
  try {
    await loading;
  } catch (error) {
    languagePromises.delete(shikiId);
    throw error;
  }
  return shikiId;
}

/* ensure the highlighter is ready AND a specific language is available.
   Returns the highlighter + the resolved shiki language id, so callers can
   run codeToTokens right after. */
export async function ensureHighlighter(lang: LangId): Promise<{ hl: HighlighterCore; shikiId: ShikiLang | null }> {
  const hl = await getHighlighter();
  const shikiId = await ensureLang(lang);
  return { hl, shikiId };
}
