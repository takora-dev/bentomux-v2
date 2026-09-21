/* Session Notes — the bundled example plugin.
 *
 * This is the platform's worked example, and it is deliberately the broadest
 * one: it touches four contribution points at once (a dock button, a modal, a
 * widget, and two commands) plus plugin storage and a host event. Anything
 * non-trivial you want to build is closer to this file than to the
 * single-purpose templates.
 *
 * It is also the only plugin shipped inside the app, so it uses the reserved
 * `bentomux.` publisher — a third-party plugin cannot.
 *
 * Three things worth copying:
 *   * Notes are keyed by workspace, so switching projects does not mix them up.
 *   * Every listener is released through ctx.dispose(), because ES modules
 *     cannot be unloaded and a disabled plugin must not keep running.
 *   * The modal re-reads storage each time it opens, rather than showing a
 *     snapshot taken at activation. */

const STORAGE_KEY = 'notes';

/** Notes are stored as { [workspaceKey]: string[] }. */
async function readNotes(ctx) {
  return (await ctx.storage.get(STORAGE_KEY)) ?? {};
}

/** Which workspace a note belongs to. Falls back to one bucket when no
 *  workspace is open, so the plugin is still usable on a fresh install. */
async function workspaceKey(ctx) {
  try {
    const state = await ctx.app.getState();
    const id = state?.activeWorkspaceId;
    return typeof id === 'string' && id ? id : 'no-workspace';
  } catch {
    /* app.read may be refused if the user edited the manifest; a note without
       a workspace is better than a broken dialog */
    return 'no-workspace';
  }
}

/** @param {import('./bentomux-plugin-sdk').PluginContext} ctx */
export function activate(ctx) {
  ctx.log('session notes ready');

  /* ---------------- the modal ---------------- */

  ctx.ui.modal('bentomux.session-notes.modal', host => {
    /* `key` tracks the active workspace for as long as the dialog is open, so
       switching workspace behind the modal does not file a note against the
       wrong project */
    let key = 'no-workspace';

    const input = document.createElement('input');
    input.type = 'text';
    input.placeholder = 'Write a note…';
    input.className = 'session-notes-input';

    const addButton = document.createElement('button');
    addButton.className = 'btn primary';
    addButton.textContent = 'Add';

    const row = document.createElement('div');
    row.className = 'session-notes-row';
    row.append(input, addButton);

    const status = document.createElement('div');
    status.className = 'session-notes-status';

    const list = document.createElement('div');
    list.className = 'session-notes-list';

    const paint = async () => {
      const notes = await readNotes(ctx);
      const items = notes[key] ?? [];
      list.textContent = '';
      if (!items.length) {
        const empty = document.createElement('div');
        empty.className = 'session-notes-empty';
        empty.textContent = 'No notes yet for this workspace.';
        list.append(empty);
        return;
      }
      /* newest first: the last thing written is what you came back for */
      for (const note of items.slice().reverse()) {
        const el = document.createElement('div');
        el.className = 'session-notes-item';
        el.textContent = note;
        list.append(el);
      }
    };

    const save = async () => {
      const text = input.value.trim();
      if (!text) return;
      const notes = await readNotes(ctx);
      notes[key] = [...(notes[key] ?? []), text];
      await ctx.storage.set(STORAGE_KEY, notes);
      input.value = '';
      status.textContent = 'Saved.';
      await paint();
      ctx.log('note saved');
    };

    addButton.addEventListener('click', () => void save());
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter') void save();
    });

    /* the workspace may change while the dialog is open */
    const off = ctx.events.on('workspace:changed', () => {
      void workspaceKey(ctx).then(next => {
        key = next;
        status.textContent = '';
        void paint();
      });
    });
    ctx.dispose(off);

    host.append(row, status, list);
    void workspaceKey(ctx).then(next => {
      key = next;
      void paint();
    });
  });

  /* ---------------- commands ---------------- */

  ctx.ui.command('bentomux.session-notes.open', () => {
    ctx.ui.openModal('bentomux.session-notes.modal');
  });

  /* A second command that lands in the same place, so the palette entry is
     discoverable under the verb a user is actually thinking of. */
  ctx.ui.command('bentomux.session-notes.add', () => {
    ctx.ui.openModal('bentomux.session-notes.modal');
  });

  /* ---------------- welcome-page widget ---------------- */

  ctx.ui.widget('bentomux.session-notes.widget', host => {
    const paint = async () => {
      const notes = await readNotes(ctx);
      const all = Object.values(notes).flat();
      host.textContent = all.length
        ? all[all.length - 1]
        : 'No notes yet. Open Session Notes from the sidebar dock.';
    };

    void paint();

    /* a note saved elsewhere should show up here without a reload */
    const off = ctx.events.on('workspace:changed', () => void paint());
    ctx.dispose(off);
  });
}

export function deactivate() {
  /* ctx.dispose() has already run every listener registered above */
}
