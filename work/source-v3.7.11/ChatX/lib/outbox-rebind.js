'use strict';
const crypto = require('crypto');
function rebindTrustedHost(store, config, request) {
  const origin = new URL(request.origin).origin;
  const pins = config.trustedHostKeys || {};
  if (!request.publicKey || pins[origin] !== request.publicKey) throw Error('Identitas HOST belum terverifikasi');
  const suffix = '|' + String(config.name || '').toLowerCase();
  const sources = Object.entries(pins).filter(([old, key]) => key === request.publicKey).map(([old]) => new URL(old).origin + suffix);
  const scope = 'host:' + crypto.createHash('sha256').update(request.publicKey).digest('hex') + suffix;
  return {...store.mergeScopes(scope, sources), scope};
}
module.exports = {rebindTrustedHost};
