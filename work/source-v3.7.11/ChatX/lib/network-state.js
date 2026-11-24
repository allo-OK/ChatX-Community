'use strict';

function normalizeIpv4Interfaces(interfaces = {}) {
  const rows = [];
  for (const [name, entries] of Object.entries(interfaces || {})) {
    for (const entry of entries || []) {
      if ((entry?.family !== 'IPv4' && entry?.family !== 4) || entry.internal || !entry.address) continue;
      rows.push({ name, address: String(entry.address), netmask: String(entry.netmask || '') });
    }
  }
  return rows.sort((a, b) => `${a.name}|${a.address}|${a.netmask}`.localeCompare(`${b.name}|${b.address}|${b.netmask}`));
}

function ipv4ToInt(value) {
  const parts = String(value || '').split('.');
  if (parts.length !== 4 || parts.some(part => !/^\d{1,3}$/.test(part) || Number(part) > 255)) return null;
  return parts.reduce((value, part) => ((value << 8) | Number(part)) >>> 0, 0);
}

function intToIpv4(value) {
  return [24, 16, 8, 0].map(shift => (value >>> shift) & 255).join('.');
}

function directedBroadcast(address, netmask) {
  const ip = ipv4ToInt(address), mask = ipv4ToInt(netmask);
  if (ip == null || mask == null || address.startsWith('169.254.')) return null;
  return intToIpv4(((ip & mask) | (~mask >>> 0)) >>> 0);
}

function broadcastTargets(interfaces = {}) {
  const targets = new Set(['255.255.255.255']);
  for (const item of normalizeIpv4Interfaces(interfaces)) {
    const broadcast = directedBroadcast(item.address, item.netmask);
    if (broadcast && broadcast !== item.address) targets.add(broadcast);
  }
  return [...targets];
}

function isPrivateIpv4(address) {
  return /^10\./.test(address) || /^192\.168\./.test(address) || /^172\.(1[6-9]|2\d|3[01])\./.test(address);
}

function selectLanIpv4ForPeer(interfaces = {}, peerAddress = '') {
  const peer = ipv4ToInt(peerAddress);
  if (peer == null) return selectLanIpv4(interfaces);
  const virtualName = /virtual|vmware|virtualbox|vEthernet|hyper-v|docker|wsl|loopback|vpn|tunnel|tailscale|zerotier|hamachi|wireguard/i;
  const physicalName = /wi-?fi|wireless|wlan|ethernet|\blan\b|^en\d|^eth\d/i;
  const rows = normalizeIpv4Interfaces(interfaces).filter(item => {
    const host = ipv4ToInt(item.address), mask = ipv4ToInt(item.netmask);
    return host != null && mask != null && isPrivateIpv4(item.address) && !item.address.startsWith('169.254.') && ((host & mask) >>> 0) === ((peer & mask) >>> 0);
  });
  rows.sort((a, b) => {
    const score = item => (virtualName.test(item.name) ? 100 : physicalName.test(item.name) ? 0 : 20);
    return score(a) - score(b) || a.name.localeCompare(b.name);
  });
  return rows[0]?.address || selectLanIpv4(interfaces);
}

function selectLanIpv4(interfaces = {}) {
  const virtualName = /virtual|vmware|virtualbox|vEthernet|hyper-v|docker|wsl|loopback|vpn|tunnel|tailscale|zerotier|hamachi|wireguard/i;
  const physicalName = /wi-?fi|wireless|wlan|ethernet|\blan\b|^en\d|^eth\d/i;
  const rows = normalizeIpv4Interfaces(interfaces)
    .filter(item => isPrivateIpv4(item.address) && !item.address.startsWith('169.254.'))
    .sort((a, b) => {
      const score = item => (virtualName.test(item.name) ? 100 : physicalName.test(item.name) ? 0 : 20);
      return score(a) - score(b) || a.name.localeCompare(b.name);
    });
  return rows[0]?.address || '127.0.0.1';
}

module.exports = { normalizeIpv4Interfaces, directedBroadcast, broadcastTargets, selectLanIpv4, selectLanIpv4ForPeer };
