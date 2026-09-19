import { execFileSync } from 'node:child_process';
import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { performance } from 'node:perf_hooks';

const rendererRoot = 'out/renderer';
const start = performance.now();
execFileSync('npm', ['run', 'build'], { stdio: 'inherit' });
const buildMs = Math.round(performance.now() - start);

function filesUnder(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(path));
    else files.push(path);
  }
  return files;
}

const files = filesUnder(rendererRoot).map(path => ({
  path,
  bytes: statSync(path).size,
}));
const totalBytes = files.reduce((total, file) => total + file.bytes, 0);
const largest = [...files].sort((a, b) => b.bytes - a.bytes).slice(0, 10);

console.log(`[perf] vite-build-ms=${buildMs}`);
console.log(`[perf] renderer-bytes=${totalBytes}`);
console.log(`[perf] renderer-files=${files.length}`);
console.log('[perf] largest-renderer-files:');
for (const file of largest) {
  console.log(`  ${file.bytes}\t${relative(rendererRoot, file.path)}`);
}
