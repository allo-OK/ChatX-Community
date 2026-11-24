'use strict';

// Run with Node against an offline copy of the legacy profile.
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const prefixes = [
  'kc-theme:', 'kc-appearance:', 'kc-conversation-prefs:',
  'kc-hidden-messages:', 'kc-chat-scroll:', 'kc-share-activity:',
];
const keys = ['kc.slither.skin', 'kc.gallery.searchScope'];

function exportSnapshot() {
  const [electron, snapshot, output, scratch] = process.argv.slice(2);
  if (!electron || !snapshot || !output || !scratch) {
    throw Error('Isi lokasi Electron, folder salinan profil, file JSON tujuan, dan folder kerja sementara.');
  }
  const source = path.join(snapshot, 'Local Storage', 'leveldb');
  if (!fs.existsSync(path.join(source, 'CURRENT'))) throw Error('Salinan Local Storage Electron tidak ditemukan.');
  const temporary = fs.mkdtempSync(path.join(path.resolve(scratch), 'kc-electron-prefs-'));
  try {
    const destination = path.join(temporary, 'Local Storage', 'leveldb');
    fs.mkdirSync(destination, { recursive: true });
    for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
      if (!entry.isFile()) throw Error('Salinan Local Storage berisi file yang tidak didukung.');
      if (entry.name !== 'LOCK') fs.copyFileSync(path.join(source, entry.name), path.join(destination, entry.name));
    }
    execFileSync(path.resolve(electron), [__filename, temporary, path.resolve(output)], {
      windowsHide: true, stdio: 'inherit', timeout: 30000,
    });
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true, maxRetries: 5 });
  }
}

async function readPreferences(app, BrowserWindow) {
    const [temporary, output] = process.argv.slice(2);
    app.setPath('userData', temporary);
    app.setPath('sessionData', temporary);
    const probe = path.join(temporary, 'preferences.html');
    fs.writeFileSync(probe, '<!doctype html><title>ChatX preferences migration</title>');
    await app.whenReady();
    const window = new BrowserWindow({ show: false, webPreferences: { sandbox: true, contextIsolation: true } });
    await window.loadFile(probe);
    const values = await window.webContents.executeJavaScript(`(() => {
      const prefixes = ${JSON.stringify(prefixes)};
      const keys = ${JSON.stringify(keys)};
      const values = Object.create(null);
      for (let index = 0; index < localStorage.length; index++) {
        const key = localStorage.key(index);
        if (keys.includes(key) || prefixes.some(prefix => key.startsWith(prefix) && key.length > prefix.length)) {
          values[key] = localStorage.getItem(key);
        }
      }
      return values;
    })()`);
    const entries = Object.entries(values);
    if (entries.length === 0) throw Error('Preferensi ChatX tidak ditemukan di salinan profil ini.');
    fs.writeFileSync(output, JSON.stringify({ format: 1, entries }), { flag: 'wx' });
    console.log(`Preferensi diekspor: ${entries.length}`);
}

if (process.versions.electron) {
  const { app, BrowserWindow } = require('electron');
  readPreferences(app, BrowserWindow).then(() => app.quit(), error => { console.error(error.message); app.exit(1); });
} else {
  try { exportSnapshot(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
