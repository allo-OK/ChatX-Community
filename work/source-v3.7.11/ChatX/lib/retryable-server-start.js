'use strict';

/**
 * Owns the server listener lifecycle for HOST election and takeover.
 * A failed bind must not remain cached as a live server instance: HOST retries
 * discovery and listener startup indefinitely after transient EADDRINUSE/network
 * startup errors.
 */
function createRetryableServerStarter({ getCurrent, setCurrent, startServer }) {
  let startFlight = null;

  async function start(options) {
    if (startFlight) return startFlight;

    const current = getCurrent();
    if (current) {
      if (current.server?.listening) {
        try {
          await current.ready;
          return current;
        } catch (_) {
          // Clear a listener whose readiness promise rejected after creation.
        }
      }
      if (getCurrent() === current) setCurrent(null);
      try { await current.shutdown?.(); } catch (_) {}
    }

    const candidate = startServer(options);
    const flight = (async () => {
      try {
        await candidate.ready;
        if (candidate.server && !candidate.server.listening) {
          throw new Error('HOST listener closed before readiness completed');
        }
        setCurrent(candidate);
        return candidate;
      } catch (error) {
        if (getCurrent() === candidate) setCurrent(null);
        try { await candidate.shutdown?.(); } catch (_) {}
        throw error;
      }
    })();
    startFlight = flight;
    try {
      return await flight;
    } finally {
      if (startFlight === flight) startFlight = null;
    }
  }

  return start;
}

module.exports = { createRetryableServerStarter };
