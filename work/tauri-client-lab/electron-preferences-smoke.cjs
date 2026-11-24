'use strict';
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const electron = require('../source-v3.7.11/ChatX/node_modules/electron');
const scratch = fs.mkdtempSync(path.join(__dirname, 'preferences-qa-'));
try {
  const profile = path.join(scratch, 'profile');
  fs.mkdirSync(profile);
  const fixture = path.join(scratch, 'seed.cjs');
  const html = path.join(scratch, 'seed.html');
  const output = path.join(scratch, 'preferences.json');
  fs.writeFileSync(html, '<!doctype html><title>Fixture</title>');
  fs.writeFileSync(fixture, `
    const { app, BrowserWindow, session } = require('electron');
    app.setPath('userData', ${JSON.stringify(profile)});
    app.setPath('sessionData', ${JSON.stringify(profile)});
    app.whenReady().then(async () => {
      const window = new BrowserWindow({ show: false });
      await window.loadFile(${JSON.stringify(html)});
      await window.webContents.executeJavaScript(\`
        localStorage.setItem('kc-theme:legacy editor', 'light');
        localStorage.setItem('kc-conversation-prefs:legacy editor', '{"pins":["general"]}');
        localStorage.setItem('kc-chat-cache-v220:legacy:general', 'private-message');
        localStorage.setItem('persist:root', 'sensitive-session');
      \`);
      session.defaultSession.flushStorageData();
      app.quit();
    }).catch(error => { console.error(error); app.exit(1); });
  `);
  const run = args => execFileSync(electron, args, { windowsHide: true, timeout: 20000 });
  run([fixture]);
  const exporter = path.join(__dirname, 'export-electron-preferences.cjs');
  const exportProfile = () => execFileSync(process.execPath, [exporter, electron, profile, output, scratch], { windowsHide: true });
  exportProfile();
  const exported = JSON.parse(fs.readFileSync(output, 'utf8'));
  assert.equal(exported.format, 1);
  assert.deepEqual(Object.fromEntries(exported.entries), {
    'kc-theme:legacy editor': 'light',
    'kc-conversation-prefs:legacy editor': '{"pins":["general"]}',
  });
  assert.throws(exportProfile, /EEXIST/);
  assert.equal(fs.readdirSync(scratch).some(name => name.startsWith('kc-electron-prefs-')), false);
  console.log('PASS: Electron profile export preserves preferences and excludes cache and sessions');
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
