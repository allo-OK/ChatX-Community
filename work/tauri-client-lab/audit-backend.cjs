'use strict';
// Static inventory. A matching route literal is evidence of wiring, not of parity.
const fs = require('node:fs');
const path = require('node:path');
const root = path.join(__dirname, '../source-v3.7.11/ChatX');
const source = fs.readFileSync(path.join(root, 'server.js'), 'utf8');
const location = (text, offset) => text.slice(0, offset).split('\n').length;
const rustRoot = path.join(__dirname, 'src-tauri/src');
const rust = fs.readdirSync(rustRoot).filter(name => name.endsWith('.rs'))
  .map(name => ({ file: 'src-tauri/src/' + name, text: fs.readFileSync(path.join(rustRoot, name), 'utf8') }));
function routeCandidates(unit) {
  const entries = [...unit.text.matchAll(/\.route\(\s*"([^"]+)"/g)]
    .map(hit => ({ path: hit[1], offset: hit.index }));
  if (unit.file.endsWith('/update_publish.rs')) {
    const definitions = [...unit.text.matchAll(/\.route\(\s*(?:prefix\b|&format!\("\{prefix\}(\/[^"]+)"\))/g)];
    for (const prefix of unit.text.matchAll(/channel_routes\(Channel::\w+,\s*"([^"]+)"\)/g)) {
      for (const route of definitions) {
        entries.push({ path: prefix[1] + (route[1] || ''), offset: route.index });
      }
    }
  }
  return entries;
}
const rustRoutes = rust.flatMap(unit => routeCandidates(unit).map(route => ({
  path: route.path, file: unit.file, line: location(unit.text, route.offset),
})));
const routes = [...source.matchAll(/\bapp\.(get|post|put|delete|patch)\(\s*(['"])(.*?)\2/g)].map(hit => {
  const normalized = hit[3].replace(/:([\w]+)/g, '{$1}');
  const candidates = rustRoutes.filter(route => route.path === normalized).map(({file,line}) => ({file,line}));
  return { method: hit[1].toUpperCase(), path: hit[3], source: 'server.js', line: location(source, hit.index),
    rustCandidates: candidates, validation: 'requires-contract-test' };
});
const socketEvents = [...source.matchAll(/\bsocket\.on\(\s*(['"])(.*?)\1/g)].map(hit => ({
  event: hit[2], source: 'server.js', line: location(source, hit.index), validation: 'requires-contract-test',
}));
const nativeChannels = [];
for (const directory of ['', 'lib']) {
  for (const file of fs.readdirSync(path.join(root, directory)).filter(file => file.endsWith('.js'))) {
    const relative = [directory, file].filter(Boolean).join('/');
    const text = fs.readFileSync(path.join(root, relative), 'utf8');
    for (const hit of text.matchAll(/(?:\bipcMain\.(?:handle|on)|\bhandle)\(\s*(['"])(.*?)\1/g)) {
      nativeChannels.push({ channel: hit[2], source: relative, line: location(text, hit.index), validation: 'requires-contract-test' });
    }
  }
}
const output = { baseline: '8.1.4', scope: 'Literal Electron routes and native channels, plus known Rust update route generation. Other dynamic registrations require manual review.',
  warning: 'A Rust candidate does not establish matching HTTP method, authorization, behavior, persistence, or UI integration. Proven results are recorded in PORT-V814.md.',
  routes, socketEvents, nativeChannels };
fs.writeFileSync(path.join(__dirname, 'backend-contract.json'), JSON.stringify(output, null, 2) + '\n');
console.log(JSON.stringify({ routes: routes.length, socketEvents: socketEvents.length, nativeChannels: nativeChannels.length,
  routesWithoutRustCandidate: routes.filter(route => !route.rustCandidates.length).length }));
