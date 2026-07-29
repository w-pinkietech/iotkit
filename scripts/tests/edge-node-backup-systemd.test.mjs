import assert from 'node:assert/strict';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

const repositoryRoot = resolve(import.meta.dirname, '../..');
const systemdDirectory = join(repositoryRoot, 'deploy', 'systemd');
const regexEscape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

test('backup service has the owner-only runtime staging contract', async () => {
  const service = await readFile(
    join(systemdDirectory, 'iotkit-edge-node-backup.service'),
    'utf8',
  );

  for (const line of [
    '[Service]',
    'Type=oneshot',
    'UMask=0077',
    'RuntimeDirectory=iotkit-edge-node-backup',
    'RuntimeDirectoryMode=0700',
    'Environment=TMPDIR=/run/iotkit-edge-node-backup',
    'ExecStart=/usr/local/bin/iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json',
  ]) {
    assert.match(service, new RegExp(`^${regexEscape(line)}$`, 'm'));
  }
});

test('backup timer is opt-in and uses the daily jitter contract', async () => {
  const timer = await readFile(
    join(systemdDirectory, 'iotkit-edge-node-backup.timer'),
    'utf8',
  );

  for (const line of [
    '[Timer]',
    'OnCalendar=daily',
    'RandomizedDelaySec=2h',
    'Persistent=true',
  ]) {
    assert.match(timer, new RegExp(`^${regexEscape(line)}$`, 'm'));
  }
  assert.match(timer, /^\[Install\]$/m);
  assert.match(timer, /^WantedBy=timers\.target$/m);
  assert.doesNotMatch(timer, /^DefaultDependencies=no$/m);
});

test('nodectl configure pins the exact captured mount point in a temporary drop-in', async (t) => {
  const nodectl = process.env.IOTKIT_NODECTL ?? join(repositoryRoot, 'target', 'debug', 'iotkit-edge-nodectl');
  if (!existsSync(nodectl) || process.platform === 'win32') {
    t.skip('Linux-only product coverage; run with a Linux nodectl binary (WSL CI); Rust backup_cli is not a Windows product substitute');
    return;
  }

  const temporaryRoot = await mkdtemp('/dev/shm/iotkit-backup-systemd-');
  const configRoot = await mkdtemp(join(tmpdir(), 'iotkit-backup-config-'));
  t.after(async () => {
    await rm(temporaryRoot, { recursive: true, force: true });
    await rm(configRoot, { recursive: true, force: true });
  });
  await chmod(configRoot, 0o700);
  const destination = join(temporaryRoot, 'destination with spaces');
  const staging = join(temporaryRoot, 'staging');
  await mkdir(destination, { mode: 0o700 });
  await mkdir(staging, { mode: 0o700 });
  const database = join(configRoot, 'edge.db');
  const passphrase = join(configRoot, 'passphrase');
  const config = join(configRoot, 'edge-node-backup.json');
  const dropIn = join(configRoot, 'iotkit-edge-node-backup.service.d.conf');
  await writeFile(database, 'fixture', { encoding: 'utf8', mode: 0o600 });
  await writeFile(passphrase, 'owner-only-test-passphrase', { encoding: 'utf8', mode: 0o600 });

  const result = spawnSync(nodectl, [
    'backup', 'configure',
    '--config', config,
    '--db', database,
    '--destination', destination,
    '--staging-directory', staging,
    '--passphrase-file', passphrase,
    '--freshness-seconds', '86400',
    '--retention-count', '7',
    '--systemd-drop-in', dropIn,
  ], { cwd: repositoryRoot, encoding: 'utf8' });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);

  const persisted = JSON.parse(await readFile(config, 'utf8'));
  const capturedMountPoint = persisted.expected_mount.mount_point;
  const configured = await readFile(dropIn, 'utf8');
  assert.equal(configured, `[Unit]\nRequiresMountsFor=${capturedMountPoint}\n`);
  assert.match(configured, new RegExp(`^RequiresMountsFor=${regexEscape(capturedMountPoint)}$`, 'm'));
});
