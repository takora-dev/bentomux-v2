/* ---------------- Plugin Creator ----------------
   Spec: docs/PLUGIN_PLATFORM.md §13–§14.

   A wizard that scaffolds a plugin from a template into a folder the user
   picks, then hands off.

   The handoff is the point. Bentomux does not write the plugin's code and
   does not pretend to: the app scaffolds, validates, and packages, while the
   intelligence is the user's own agent working on the folder with its own
   tools. So the last screen of this wizard is a prompt to paste into an
   agent, not a code editor. */

import { h } from '../dom';
import api from '../../preload/bentomux';
import { openModal } from '../components/modal';
import { readableError, showRestartNotice } from './pluginStudio';
import type { ValidationReport } from '../plugin/types';

interface TemplateInfo {
  name: string;
  contributes: string[];
}

/** What each template adds, for the picker. Keys match the template folders. */
const TEMPLATE_BLURB: Record<string, string> = {
  basic: 'The minimum: a manifest, an entry module, and one command.',
  sidebar: 'Adds a row to the sidebar that fires a command.',
  topbar: 'Adds a button to the titlebar that fires a command.',
  modal: 'Adds a dialog, opened from a command.',
  tab: 'Adds a workspace tab that stays open alongside your terminals.',
  widget: 'Adds a card to the welcome page. Uses plugin storage.',
  service: 'Adds a background task that runs while the app is open.',
};

interface WizardState {
  template: string;
  dest: string;
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
}

export function openCreator(): void {
  void runCreator();
}

async function runCreator(): Promise<void> {
  let templates: TemplateInfo[];
  try {
    templates = await api.pluginTemplates();
  } catch (e) {
    showError('Could not read the templates', readableError(e));
    return;
  }
  if (!templates.length) {
    showError(
      'No templates available',
      'The plugin templates are not bundled with this build of Bentomux.',
    );
    return;
  }

  const template = await pickTemplate(templates);
  if (!template) return;

  const state = await fillDetails(template);
  if (!state) return;

  await scaffoldAndHandOff(state);
}

/* ---------------- step 1: template ---------------- */

function pickTemplate(templates: TemplateInfo[]): Promise<TemplateInfo | null> {
  return new Promise(resolve => {
    let settled = false;
    const finish = (v: TemplateInfo | null): void => {
      if (settled) return;
      settled = true;
      modal.close();
      resolve(v);
    };

    const modal = openModal({
      title: 'Create a plugin',
      body: h('div', { class: 'creator-templates' },
        h('p', { class: 'creator-lede' }, 'Start from a template. You can change anything afterwards.'),
        ...templates.map(t => h('button', {
          class: 'creator-template',
          type: 'button',
          onclick: () => finish(t),
        },
          h('span', { class: 'creator-template-name' }, t.name),
          h('span', { class: 'creator-template-desc' },
            TEMPLATE_BLURB[t.name] ?? `Contributes: ${t.contributes.join(', ') || 'nothing yet'}`),
        )),
      ),
      onClose: () => finish(null),
    });
  });
}

/* ---------------- step 2: details ---------------- */

function fillDetails(template: TemplateInfo): Promise<WizardState | null> {
  return new Promise(resolve => {
    let settled = false;
    const finish = (v: WizardState | null): void => {
      if (settled) return;
      settled = true;
      modal.close();
      resolve(v);
    };

    const idInput = h('input', { type: 'text', placeholder: 'acme.my-plugin' }) as HTMLInputElement;
    const nameInput = h('input', { type: 'text', placeholder: 'My Plugin' }) as HTMLInputElement;
    const versionInput = h('input', { type: 'text', value: '1.0.0' }) as HTMLInputElement;
    const descInput = h('input', { type: 'text', placeholder: 'What this plugin does' }) as HTMLInputElement;
    const authorInput = h('input', { type: 'text', placeholder: 'Your name' }) as HTMLInputElement;
    const destLabel = h('span', { class: 'creator-dest' }, 'No folder chosen');

    const problem = h('div', { class: 'creator-problem' });
    let dest = '';

    const chooseFolder = h('button', {
      class: 'btn',
      type: 'button',
      onclick: () => {
        void api.pluginChooseNewFolder().then(path => {
          if (!path) return;
          dest = path;
          destLabel.textContent = path;
          /* prefill the plugin id from the folder name when it looks usable */
          if (!idInput.value) {
            const base = path.split(/[\\/]/).filter(Boolean).pop() ?? '';
            const slug = base.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-|-$/g, '');
            if (slug) idInput.value = `local.${slug}`;
          }
          if (!nameInput.value) {
            const base = path.split(/[\\/]/).filter(Boolean).pop() ?? '';
            if (base) nameInput.value = base;
          }
        });
      },
    }, 'Choose folder…');

    const create = h('button', {
      class: 'btn primary',
      type: 'button',
      onclick: () => {
        const id = idInput.value.trim();
        const name = nameInput.value.trim();
        const version = versionInput.value.trim();

        const issue = validateDetails(id, name, version, dest);
        if (issue) {
          problem.textContent = issue;
          return;
        }
        finish({ template: template.name, dest, id, name, version, description: descInput.value.trim(), author: authorInput.value.trim() });
      },
    }, 'Create');

    const modal = openModal({
      title: 'New plugin details',
      body: h('div', { class: 'creator-form' },
        h('div', { class: 'creator-field' },
          h('label', {}, 'Plugin id'),
          idInput,
          h('span', { class: 'creator-hint' }, 'publisher.name — lowercase letters, digits, and dashes'),
        ),
        h('div', { class: 'creator-field' }, h('label', {}, 'Display name'), nameInput),
        h('div', { class: 'creator-field' }, h('label', {}, 'Version'), versionInput),
        h('div', { class: 'creator-field' }, h('label', {}, 'Description'), descInput),
        h('div', { class: 'creator-field' }, h('label', {}, 'Author'), authorInput),
        h('div', { class: 'creator-field' }, h('label', {}, 'Folder'), chooseFolder, destLabel),
        problem,
      ),
      footer: h('div', { class: 'plugin-review-foot' },
        h('button', { class: 'btn', type: 'button', onclick: () => finish(null) }, 'Cancel'),
        create,
      ),
      onClose: () => finish(null),
    });

    setTimeout(() => idInput.focus(), 0);
  });
}

/** Client-side checks mirror the validator's, so the user hears about a typo
 *  before a folder is created rather than after. */
function validateDetails(id: string, name: string, version: string, dest: string): string | null {
  if (!dest) return 'Choose a folder for the plugin.';
  if (!id) return 'Give the plugin an id.';
  const parts = id.split('.');
  const seg = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/;
  if (parts.length !== 2 || !parts.every(p => seg.test(p))) {
    return 'The id must be `publisher.name` — two parts, lowercase letters, digits, and dashes only.';
  }
  if (id.startsWith('bentomux.')) {
    return 'The `bentomux` publisher is reserved for plugins that ship with the app.';
  }
  if (!name) return 'Give the plugin a display name.';
  if (!/^\d+\.\d+\.\d+/.test(version)) return 'Version must look like 1.0.0.';
  return null;
}

/* ---------------- step 3: scaffold and hand off ---------------- */

async function scaffoldAndHandOff(state: WizardState): Promise<void> {
  let report: ValidationReport;
  try {
    report = await api.pluginScaffold({
      template: state.template,
      dest: state.dest,
      id: state.id,
      name: state.name,
      version: state.version,
      description: state.description,
      author: state.author,
    });
  } catch (e) {
    showError('Could not create the plugin', readableError(e));
    return;
  }

  if (!report.ok) {
    /* the scaffold validates itself; a failure here is a bug in the template,
       not something the user did */
    showError(
      'The generated plugin did not validate',
      report.errors.map(e => `${e.code}: ${e.message}`).join('\n') +
      '\n\nThis is a bug in the template, not in what you entered. Please report it.',
    );
    return;
  }

  showHandoff(state, report);
}

function showHandoff(state: WizardState, report: ValidationReport): void {
  const prompt =
    `Implement the plugin in ${state.dest} using the bentomux-plugin-author skill.\n` +
    `It is a Bentomux plugin scaffolded from the "${state.template}" template.\n` +
    `Run \`bentomux --plugin-validate .\` until it passes.`;

  const promptBox = h('textarea', { class: 'creator-prompt', readonly: 'true', rows: '4' }) as HTMLTextAreaElement;
  promptBox.value = prompt;

  const copy = h('button', {
    class: 'btn',
    type: 'button',
    onclick: () => {
      void navigator.clipboard.writeText(prompt).then(() => {
        copy.textContent = 'Copied';
        setTimeout(() => { copy.textContent = 'Copy prompt'; }, 1500);
      }).catch(() => {
        /* clipboard can be denied; the textarea is selectable either way */
        promptBox.select();
      });
    },
  }, 'Copy prompt');

  const install = h('button', {
    class: 'btn primary',
    type: 'button',
    onclick: () => {
      void api.pluginInstallFolder(state.dest)
        .then(record => api.pluginSetEnabled(record.id, true))
        .then(() => {
          modal.close();
          /* same reason as the Studio install flow: the plugin list is read at
             boot, so a plugin installed now needs a restart to appear */
          showRestartNotice(state.name);
        })
        .catch(e => showError('Could not install it yet', readableError(e)));
    },
  }, 'Install now');

  const modal = openModal({
    title: 'Plugin created',
    body: h('div', { class: 'creator-handoff' },
      h('div', { class: 'plugin-review-id' }, state.id + ' · ' + state.dest),
      h('p', {}, 'The scaffold is on disk and validates. Now let your agent write it:'),
      promptBox,
      h('div', { class: 'creator-actions' }, copy),
      h('p', { class: 'plugin-review-note' },
        'Bentomux does not write the plugin code itself. Your agent has the skill, ' +
        'the templates, and the validator — that is the whole loop.'),
      report.warnings.length
        ? h('p', { class: 'plugin-review-note' },
            'Validator notes: ' + report.warnings.map(w => w.message).join('; '))
        : null,
    ),
    footer: h('div', { class: 'plugin-review-foot' },
      h('button', { class: 'btn', type: 'button', onclick: () => modal.close() }, 'Close'),
      install,
    ),
  });
}

function showError(title: string, detail: string): void {
  openModal({
    title,
    body: h('div', { class: 'plugin-report' }, h('p', {}, detail)),
  });
}
