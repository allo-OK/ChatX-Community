const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const script = fs.readFileSync(path.join(__dirname, 'host-renderer-bridge.js'), 'utf8');
const bootstrap = script.slice(script.indexOf('  const read ='), script.indexOf('  const rows ='));
const store = new Map([
    ['kc-rust-host-profile', JSON.stringify({name: 'HOST Lab', deviceId: 'test-id', legacyDesktopProfileImported: true})],
    ['kc-rust-host-outbox', JSON.stringify([{scope: 'local-test'}])],
]);
const result = vm.runInNewContext(bootstrap + '\n({profile, profileKey, outboxKey});', {
    localStorage: {getItem: key => store.get(key), setItem: (key, value) => store.set(key, value)},
    window: {__kcLegacyDesktopProfile: {electronHostProfileId: 'existing-host', name: 'Nama HOST Electron'}},
    crypto: require('node:crypto').webcrypto,
    encodeURIComponent,
});
assert.equal(result.profile.name, 'Nama HOST Electron');
assert.notEqual(result.profile.deviceId, 'test-id');
assert.equal(result.profileKey, 'kc-rust-host-profile:existing-host');
assert.equal(result.outboxKey, 'kc-rust-host-outbox:existing-host');
assert.equal(JSON.parse(store.get('kc-rust-host-profile')).name, 'HOST Lab');
console.log('PASS: profil HOST Electron tetap memakai nama anggota meski WebView berisi profil Rust lama');
