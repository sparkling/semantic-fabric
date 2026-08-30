// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import { createServer, createConnection, type Socket } from 'node:net';

export async function launchNativeProxy(argv: readonly string[]): Promise<number> {
  const separator = argv.indexOf('--');
  if (separator !== 2 || argv[0] !== '--broker-socket' || argv[1].length === 0
    || argv.length <= separator + 1 || argv.some((value) => value.includes('\0'))) {
    throw new Error('HARNESS_NATIVE_PROXY_LAUNCH_ARGUMENT_INVALID');
  }
  const socketPath = argv[1];
  const executable = argv[separator + 1];
  const args = argv.slice(separator + 2);
  const connections = new Set<Socket>();
  const proxy = createServer((client) => {
    connections.add(client);
    client.once('close', () => connections.delete(client));
    const broker = createConnection(socketPath);
    connections.add(broker);
    broker.once('close', () => connections.delete(broker));
    client.pipe(broker);
    broker.pipe(client);
    broker.once('error', () => client.destroy());
    client.once('error', () => broker.destroy());
  });
  await new Promise<void>((resolveReady, reject) => {
    proxy.once('error', reject);
    proxy.listen(0, '127.0.0.1', () => resolveReady());
  });
  const address = proxy.address();
  if (address === null || typeof address === 'string') throw new Error('HARNESS_NATIVE_PROXY_BIND_FAILED');
  const proxyUrl = `http://127.0.0.1:${address.port}`;
  const child = spawn(executable, [...args], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      HTTP_PROXY: proxyUrl,
      HTTPS_PROXY: proxyUrl,
      ALL_PROXY: proxyUrl,
      http_proxy: proxyUrl,
      https_proxy: proxyUrl,
      all_proxy: proxyUrl,
      NO_PROXY: '',
      no_proxy: '',
    },
    shell: false,
    stdio: 'inherit',
  });
  const forward = (signal: NodeJS.Signals) => {
    try { child.kill(signal); } catch { /* child already exited */ }
  };
  process.once('SIGTERM', forward);
  process.once('SIGINT', forward);
  const exitCode = await new Promise<number>((resolveExit, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolveExit(code ?? signalExit(signal)));
  });
  process.off('SIGTERM', forward);
  process.off('SIGINT', forward);
  for (const connection of connections) connection.destroy();
  await new Promise<void>((resolveClosed) => proxy.close(() => resolveClosed()));
  return exitCode;
}

function signalExit(signal: NodeJS.Signals | null): number {
  if (signal === 'SIGINT') return 130;
  if (signal === 'SIGTERM') return 143;
  if (signal === 'SIGKILL') return 137;
  return 1;
}

// The explicit .cts/.cjs format remains unambiguous after this launcher is
// mounted into a package-less filesystem namespace by the native boundary.
if (require.main === module) {
  launchNativeProxy(process.argv.slice(2)).then(
    (code) => { process.exitCode = code; },
    (error: unknown) => {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
      process.exitCode = 1;
    },
  );
}
