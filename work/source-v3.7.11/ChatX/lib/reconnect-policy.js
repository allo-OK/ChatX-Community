'use strict';

function discoveryRetryDelay(attempt, random = Math.random) {
  const step = Math.max(0, Math.min(4, Number(attempt) || 0));
  const base = Math.min(8000, 1000 * (2 ** step));
  const sample = Math.max(0, Math.min(1, Number(random()) || 0));
  const jitter = 0.8 + sample * 0.4;
  return Math.round(base * jitter);
}

function withDeadline(task, timeout, label = 'Operasi') {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(label + ' melewati batas waktu')), timeout);
    Promise.resolve().then(() => typeof task === 'function' ? task() : task).then(
      value => { clearTimeout(timer); resolve(value); },
      error => { clearTimeout(timer); reject(error); }
    );
  });
}
const reconnectPolicy = { discoveryRetryDelay, withDeadline };
if (typeof module !== 'undefined' && module.exports) module.exports = reconnectPolicy;
if (typeof window !== 'undefined') window.KCReconnectPolicy = reconnectPolicy;
