import { spawn } from 'node:child_process';

const command = process.platform === 'win32' ? 'tauri.cmd' : 'tauri';
const child = spawn(command, ['dev', '--config', 'src-tauri/tauri.dev.conf.json'], {
  env: { ...process.env, BENTOMUX_USER_DATA_SUFFIX: '-dev' },
  stdio: 'inherit',
});

child.on('error', (error) => {
  console.error(`Failed to start Tauri dev: ${error.message}`);
  process.exitCode = 1;
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exitCode = code ?? 1;
});
