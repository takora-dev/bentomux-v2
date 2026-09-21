/* Validates every plugin template by substituting its placeholders and running
 * the real validator over the result.
 *
 * Run: node scripts/check-plugin-templates.mjs
 *
 * This is the CI gate for the plugin platform's authoring surfaces. It exists
 * because a template that does not validate is worse than no template: it
 * teaches an author (or an agent) a shape the app will reject, and the failure
 * only shows up after they have written code against it.
 *
 * It runs the *Rust* validator, not a reimplementation, so a template passing
 * here means the app will accept it. The binary is built on demand.
 *
 * One artifact, three jobs: a template is the wizard's scaffold, this CI
 * fixture, and the example an authoring agent reads. */

import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const TEMPLATES = resolve('resources/plugin-templates');
/** Bundled plugins validate with --bundled: they may use the reserved
 *  `bentomux.` publisher that a third-party plugin cannot. */
const BUNDLED = resolve('resources/plugin-bundled');
const BIN = resolve('src-tauri/target/debug/bentomux');

function findBinary() {
  const candidates = [BIN, BIN + '.exe'];
  for (const c of candidates) {
    try {
      readFileSync(c);
      return c;
    } catch {
      /* try the next candidate */
    }
  }
  return null;
}

function ensureBinary() {
  let bin = findBinary();
  if (bin) return bin;
  console.log('building the validator binary…');
  execFileSync('cargo', ['build', '--bin', 'bentomux'], {
    cwd: resolve('src-tauri'),
    stdio: 'inherit',
  });
  bin = findBinary();
  if (!bin) throw new Error('could not build src-tauri/target/debug/bentomux');
  return bin;
}

/** Fill in the placeholders the Creator wizard would ask the user for. */
function substitute(dir, values) {
  for (const file of readdirSync(dir)) {
    if (file === 'bentomux-plugin-sdk.d.ts') continue;
    const path = join(dir, file);
    let text = readFileSync(path, 'utf8');
    for (const [key, value] of Object.entries(values)) {
      text = text.split(`{{${key}}}`).join(value);
    }
    writeFileSync(path, text);
  }
}

const bin = ensureBinary();
const work = mkdtempSync(join(tmpdir(), 'bentomux-templates-'));
let failures = 0;

function check(label, dir, extraArgs) {
  const run = spawnSync(bin, ['--plugin-validate', dir, '--json', ...extraArgs], { encoding: 'utf8' });
  const report = safeParse(run.stdout);
  if (run.status === 0 && report?.ok) {
    const warnings = report.warnings?.length ?? 0;
    console.log(`  ok    ${label}${warnings ? `  (${warnings} warning${warnings === 1 ? '' : 's'})` : ''}`);
    for (const w of report.warnings ?? []) {
      console.log(`          warning [${w.code}] ${w.message}`);
    }
    return;
  }
  failures++;
  console.error(`  FAIL  ${label}  (exit ${run.status})`);
  for (const e of report?.errors ?? []) {
    console.error(`          error   [${e.code}] ${e.message}`);
  }
  if (!report) console.error('          ' + (run.stdout || run.stderr || '').trim());
}

try {
  const names = readdirSync(TEMPLATES).filter(n => {
    try {
      readFileSync(join(TEMPLATES, n, 'plugin.json'));
      return true;
    } catch {
      return false;
    }
  });

  if (!names.length) {
    console.error('no templates found under ' + TEMPLATES);
    process.exit(1);
  }

  for (const name of names) {
    const dir = join(work, name);
    cpSync(join(TEMPLATES, name), dir, { recursive: true });

    substitute(dir, {
      id: `acme.${name}-demo`,
      name: `${name[0].toUpperCase()}${name.slice(1)} Demo`,
      version: '1.0.0',
      description: `Template check for the ${name} scaffold.`,
      author: 'CI',
    });

    check(name, dir, []);
  }

  /* bundled plugins are part of the app and must validate too, with the
     reserved-publisher allowance they ship under */
  for (const name of readdirSync(BUNDLED)) {
    const dir = join(BUNDLED, name);
    try {
      readFileSync(join(dir, 'plugin.json'));
    } catch {
      continue;
    }
    check(`bundled/${name}`, dir, ['--bundled']);
  }
} finally {
  rmSync(work, { recursive: true, force: true });
}

function safeParse(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

if (failures) {
  console.error(`\n${failures} template(s) failed validation`);
  process.exit(1);
}
console.log('\nall templates validate');
