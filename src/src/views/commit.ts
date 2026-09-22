/* ---------------- commit detail page ----------------
   Opened by clicking a row in the commit graph. Shows what `git show` would:
   the decorations, the message, the parents, and the changed files.

   Each file's patch is collapsed until clicked, and its DOM is built on first
   expand. That is not a micro-optimisation — expanding a patch runs Shiki over
   every line, and a twenty-file commit should not tokenize all of them because
   the user clicked one commit. */

import { h } from '../dom';
import { closeTab } from './tabs';
import { go } from '../router';
import { ui } from '../state';
import { rel, abs } from '../time';
import type { GitCommitDetail, GitCommitFile, GitRef } from '../../shared/types';
import { diffBody, errorBody } from './diff';
import { formatStatusBadge, fileRowTitle } from './gitPanel';
import api from '../../preload/bentomux';

function refChip(ref: GitRef): HTMLElement {
  const cls = 'git-ref git-ref-' + ref.kind + (ref.head ? ' git-ref-head' : '');
  const what = ref.kind === 'tag' ? 'Tag' : ref.kind === 'remote' ? 'Remote branch' : 'Branch';
  return h('span', { class: cls, title: `${what} ${ref.name}` }, ref.name);
}

/* one changed file: a summary row that toggles its patch underneath */
function fileEntry(file: GitCommitFile): HTMLElement {
  const slot = h('div', { class: 'commit-file-patch', hidden: true });
  let built = false;

  const caret = h('span', { class: 'commit-file-caret' }, '▸');
  const row = h('button', {
    class: 'commit-file-row',
    type: 'button',
    title: fileRowTitle(file.status) + ' — ' + file.path,
    onclick: () => {
      const opening = slot.hasAttribute('hidden');
      if (opening && !built) {
        /* a binary file's block has no hunks, so the diff renderer would say
           "No changes." — which reads as a bug rather than as "binary" */
        slot.append(file.patch.includes('Binary files')
          ? h('div', { class: 'diff-editor diff-empty' }, 'Binary file — no text diff.')
          : diffBody(file, file.path));
        built = true;
      }
      if (opening) slot.removeAttribute('hidden');
      else slot.setAttribute('hidden', '');
      caret.textContent = opening ? '▾' : '▸';
      row.classList.toggle('open', opening);
    },
  },
    caret,
    h('span', { class: 'xy' }, formatStatusBadge(file.status)),
    h('span', { class: 'name' }, file.path),
    h('span', { class: 'commit-file-stat' },
      h('span', { class: 'add' }, '+' + file.additions),
      h('span', { class: 'del' }, '-' + file.deletions)));

  return h('div', { class: 'commit-file' }, row, slot);
}

function parentLinks(detail: GitCommitDetail, workspaceId: string): HTMLElement | null {
  if (!detail.parents.length) return null;
  return h('div', { class: 'commit-parents' },
    h('span', { class: 'commit-label' }, detail.parents.length > 1 ? 'Parents' : 'Parent'),
    ...detail.parents.map(p => h('button', {
      class: 'commit-parent',
      type: 'button',
      title: p,
      onclick: () => {
        go({ view: 'commit', workspaceId, oid: p, short: p.slice(0, 7) });
      },
    }, p.slice(0, 7))));
}

function detailBody(detail: GitCommitDetail, workspaceId: string): HTMLElement {
  const root = h('div', { class: 'commit-body' });

  if (detail.refs.length) {
    root.append(h('div', { class: 'commit-refs' }, ...detail.refs.map(refChip)));
  }

  if (detail.body) root.append(h('pre', { class: 'commit-message' }, detail.body));

  const parent = parentLinks(detail, workspaceId);
  if (parent) root.append(parent);

  /* a merge is diffed against its first parent (see git.rs::detail), which is
     worth saying out loud — otherwise "1 file changed" on a merge looks wrong */
  if (detail.parents.length > 1) {
    root.append(h('p', { class: 'commit-note' }, 'Merge commit — shown against its first parent.'));
  }

  root.append(h('div', { class: 'commit-summary' },
    h('span', {}, detail.files.length + (detail.files.length === 1 ? ' file changed' : ' files changed')),
    h('span', { class: 'add' }, '+' + detail.additions),
    h('span', { class: 'del' }, '-' + detail.deletions)));

  if (!detail.files.length) {
    root.append(h('div', { class: 'diff-editor diff-empty' }, 'No file changes.'));
    return root;
  }
  for (const file of detail.files) root.append(fileEntry(file));
  return root;
}

function head(detail: GitCommitDetail | null, short: string, onBack: () => void): HTMLElement {
  const sub = detail
    ? `${detail.short} · ${detail.author} <${detail.email}> · ${abs(detail.timestamp * 1000)} (${rel(detail.timestamp * 1000)})`
    : short;
  return h('div', { class: 'page-head diff-head' },
    h('button', { class: 'btn ghost back-btn', type: 'button', onclick: onBack, 'aria-label': 'Back', title: 'Back' },
      h('span', { style: 'font-size:14px' }, '←')),
    h('div', { class: 'grow' },
      h('h1', { class: 'pg commit-subject', title: detail?.subject || short }, detail?.subject || 'Commit ' + short),
      h('p', { class: 'resource-lede' }, sub)));
}

/* `short` comes from the route so the tab and the header can render before the
   fetch resolves — the oid alone would show a bare sha for a moment */
export function commitPage(workspaceId: string, oid: string, short: string): HTMLElement {
  const wrap = h('div', { class: 'page diff-page commit-page' });
  const slot = h('div', {});
  const onBack = (): void => {
    if (ui.activeTab && ui.route.view === 'commit') void closeTab(ui.activeTab);
    else go({ view: 'welcome' });
  };
  wrap.append(head(null, short, onBack), slot);

  void (async () => {
    try {
      const detail = await api.gitShow(workspaceId, oid);
      /* the header is rebuilt in place so the subject and author appear without
         re-mounting the page (which would drop the tab's scroll position) */
      wrap.replaceChild(head(detail, short, onBack), wrap.firstElementChild as Node);
      slot.innerHTML = '';
      if (!detail.isRepo) {
        slot.append(errorBody('Not a git repository.'));
        return;
      }
      slot.append(detailBody(detail, workspaceId));
    } catch (e: unknown) {
      slot.innerHTML = '';
      slot.append(errorBody(e instanceof Error ? e.message : String(e)));
    }
  })();

  return wrap;
}
