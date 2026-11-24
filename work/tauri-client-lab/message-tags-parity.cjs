'use strict';
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const sourceRoot = path.resolve(__dirname, '../source-v3.7.11/ChatX');
const source = fs.readFileSync(path.join(sourceRoot, 'server.js'), 'utf8');
const policyFunctions = ['profileKey','defaultPerms','defaultTagSpamPolicy','normalizeTagSpamPolicy',
  'normalizeModerationRole','moderationRoleDefaults','normalizeMentionLevel','normalizeFullscreenMode','cleanCohost'];
const definitions = policyFunctions.map(name => {
  const line = source.split('\n').find(line => line.trimStart().startsWith('function ' + name + '('));
  if (!line) throw Error('Original policy function changed: ' + name);
  return line;
}).join('\n');
const userPolicySource = source.slice(source.indexOf('  function userPolicy('), source.indexOf('  function featureAllowed('));
const guardSource = source.slice(source.indexOf('  function guardTaggedNotification('), source.indexOf('  const pendingGalleryPreview='));
if (!userPolicySource || !guardSource) throw Error('Original policy boundaries changed');
const cases = [
  { name: 'Editor Lab', device: 'pc1', host: false, moderation: {} },
  { name: 'HOST Lab', device: 'host', host: true, moderation: {} },
  { name: 'Editor Lab', device: 'pc1', host: false, moderation: { profilePolicies: { 'editor-lab': { tagVideoEnabled: true } } } },
  { name: 'Editor Lab', device: 'pc1', host: false, moderation: { users: { pc1: { permissions: { everyone: true }, bypassWindowsMute: true } },
    profilePolicies: { 'editor-lab': { permissions: { personal: false }, tagVideoEnabled: false } } } },
  { name: 'Admin Lab', device: 'pc1', host: false, moderation: { profilePolicies: { 'admin-lab': { moderationRole: 'ADMIN' } } } },
  { name: 'Admin Lab', device: 'pc1', host: false, moderation: { profilePolicies: { 'admin-lab': { moderationRole: 'ADMIN' } }, roles: { ADMIN: { permissions: { personal: true } } } } },
  { name: 'QC Lab', device: 'pc1', host: false, moderation: { roles: { 'role-qc': { name: 'QC Senior', permissions: { editor: true, sendFiles: false },
    mentionLevel: 'fullscreen', fullscreenMode: 'video' } }, profilePolicies: { 'qc-lab': { moderationRole: 'CUSTOM', roleTemplateId: 'role-qc' } } } },
  { name: 'Editor Lab', device: 'pc1', host: false, moderation: { profilePolicies: { 'editor-lab': { mentionLevel: 'invalid', fullscreenMode: 'invalid', mutedUntil: 123456 } } } },
];
for (const item of cases) {
  const context = { ...require(path.join(sourceRoot, 'lib/feature-policy.js')), cohostKeys: ['manageUsers','manageVideos','manageJobs','viewDiagnostics'],
    moderation: { users: {}, profilePolicies: {}, roles: {}, ...item.moderation } };
  item.expected = JSON.parse(JSON.stringify(vm.runInNewContext(definitions + '\n' + userPolicySource + '\nuserPolicy(' +
    JSON.stringify(item.device) + ',' + JSON.stringify(item.host) + ',' + JSON.stringify(item.name) + ')', context)));
  delete item.expected.cohost;
}
const sequences = [
  { policy: {}, events: [0,1,2,3,30002,60001].map(at => ({ at, tag: true, mass: false, names: [] })) },
  { policy: {}, events: [0,19999,20001,50000,60001].map(at => ({ at, tag: false, mass: true, names: [] })) },
  { policy: { maxPer10Seconds: 10, maxPerMinute: 2, blockSeconds: 5 }, events: [0,1,2,5002,60002].map(at => ({ at, tag: false, mass: false, names: ['Target'] })) },
  { policy: { maxPer10Seconds: null, maxPerMinute: '3', blockSeconds: '5' }, events: [0,1,5001,60001].map(at => ({ at, tag: true, mass: false, names: [] })) },
];
for (const item of sequences) {
  let now = 1000000;
  const context = { Date: { now: () => now }, moderation: { tagSpamPolicy: item.policy }, tagSpamBuckets: new Map(), addAudit() {} };
  const guard = vm.runInNewContext(definitions.split('\n').filter(line => line.includes('function normalizeTagSpamPolicy')).join('\n') + '\n' + guardSource + '\nguardTaggedNotification', context);
  for (const event of item.events) {
    now = 1000000 + event.at;
    event.expected = guard({ deviceId: 'pc1', name: 'Sender' }, event.tag ? 'print-test' : null,
      { everyone: event.mass, editor: false, names: event.names }) || null;
  }
}
fs.writeFileSync(path.join(__dirname, 'message-tags-fixtures.json'), JSON.stringify({ cases, sequences }, null, 2) + '\n');
console.log('Generated policy and throttle expectations directly from original v8.1.4 functions');
