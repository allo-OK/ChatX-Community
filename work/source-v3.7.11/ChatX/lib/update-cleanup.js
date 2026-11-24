'use strict';
const fs = require('fs');
const path = require('path');

const BUILD_OUTPUTS = Object.freeze({
  all: ['dist', 'dist-host', 'dist-client', 'release', 'out'],
  host: ['dist-host'],
  client: ['dist-client'],
  builder: ['dist-host', 'dist-client', 'release', 'out']
});
const CLIENT_INSTALLER_RE = /^ChatX-Chat-CLIENT-SETUP-(?:v[0-9A-Za-z._-]+-)?\d+\.exe$/i;
const ARCHIVE_EXE_RE = /^\d+_[0-9A-Za-z._-]+\.exe$/i;

function cleanupBuildOutputs(projectRoot, scope = 'all') {
  const names = BUILD_OUTPUTS[scope];
  if (!names) throw new Error(`Scope cleanup tidak dikenal: ${scope}`);
  const root = path.resolve(projectRoot);
  const removed = [];
  for (const name of names) {
    const target = path.resolve(root, name);
    if (path.dirname(target) !== root) throw new Error('Path output build tidak aman');
    try {
      if (fs.existsSync(target)) {
        fs.rmSync(target, { recursive: true, force: true, maxRetries: 3, retryDelay: 150 });
        removed.push(name);
      }
    } catch (error) {
      throw new Error(`Gagal membersihkan ${name}: ${error.message}`);
    }
  }
  return removed;
}

function cleanupClientInstallerDownloads(downloadDir, options = {}) {
  const opts = typeof options === 'number' ? { keep: options } : (options || {});
  const keep = Math.max(0, Math.floor(Number(opts.keep ?? 1) || 0));
  const currentVersion = String(opts.currentVersion || '');
  const root = path.resolve(downloadDir);
  if (!fs.existsSync(root)) return { kept: 0, removed: [] };
  const installers = [];
  for (const name of fs.readdirSync(root)) {
    if (!CLIENT_INSTALLER_RE.test(name)) continue;
    const full = path.join(root, name);
    try {
      const stat = fs.lstatSync(full);
      if (!stat.isFile()) continue;
      const match = name.match(/-v([0-9]+\.[0-9]+\.[0-9]+)-\d+\.exe$/i);
      installers.push({ name, full, mtime: stat.mtimeMs, version: match?.[1] || '' });
    } catch {}
  }
  installers.sort((a, b) => b.mtime - a.mtime || b.name.localeCompare(a.name));
  const versionCompare = (a, b) => {
    const aa = a.split('.').map(Number), bb = b.split('.').map(Number);
    for (let i = 0; i < 3; i++) if ((aa[i] || 0) !== (bb[i] || 0)) return (aa[i] || 0) - (bb[i] || 0);
    return 0;
  };
  const candidates = installers.filter(x => !currentVersion || !x.version || versionCompare(x.version, currentVersion) > 0);
  const retained = new Set(candidates.slice(0, keep).map(x => x.full));
  const removed = [];
  for (const item of installers) {
    if (retained.has(item.full)) continue;
    try { fs.unlinkSync(item.full); removed.push(item.name); } catch {}
  }
  return { kept: retained.size, removed };
}

function prunePublishedUpdateArchive(archiveDir, keep = 5) {
  const root = path.resolve(archiveDir);
  if (!fs.existsSync(root)) return { kept: 0, removed: [] };
  const rows = [];
  for (const name of fs.readdirSync(root)) {
    if (!name.endsWith('.json')) continue;
    const metaPath = path.join(root, name);
    try {
      const meta = JSON.parse(fs.readFileSync(metaPath, 'utf8'));
      const exeName = path.basename(String(meta.exe || ''));
      if (!ARCHIVE_EXE_RE.test(exeName) || exeName.replace(/\.exe$/i, '.json') !== name) continue;
      const exePath = path.join(root, exeName);
      if (!fs.statSync(exePath).isFile()) continue;
      rows.push({ name, metaPath, exeName, exePath, publishedAt: Number(meta.publishedAt || 0) });
    } catch {}
  }
  rows.sort((a, b) => b.publishedAt - a.publishedAt || b.name.localeCompare(a.name));
  const count = Math.max(0, Math.floor(Number(keep) || 0));
  const removed = [];
  for (const row of rows.slice(count)) {
    try { fs.unlinkSync(row.metaPath); } catch {}
    try { fs.unlinkSync(row.exePath); } catch {}
    removed.push(row.exeName);
  }
  return { kept: Math.min(rows.length, count), removed };
}

module.exports = { cleanupBuildOutputs, cleanupClientInstallerDownloads, prunePublishedUpdateArchive };
