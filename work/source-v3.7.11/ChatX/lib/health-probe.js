'use strict';
const http = require('http');
function probeHealth(base, timeout = 420) {
  return new Promise(resolve => {
    let request, response, settled = false;
    const finish = ok => {
      if (settled) return; settled = true; clearTimeout(timer);
      response?.destroy(); request?.destroy(); resolve(ok);
    };
    const timer = setTimeout(() => finish(false), timeout);
    try {
      const url = new URL('/health', base);
      if (url.protocol !== 'http:') return finish(false);
      request = http.get(url, res => {
        response = res; let body = '';
        res.on('error', () => finish(false));
        res.on('data', chunk => { body += chunk; if (body.length > 16384) finish(false); });
        res.on('end', () => { try { const data = JSON.parse(body); finish(res.statusCode === 200 && data?.service === 'CHATX_COMMUNITY'); } catch { finish(false); } });
      });
      request.on('error', () => finish(false));
    } catch { finish(false); }
  });
}
module.exports = {probeHealth};
