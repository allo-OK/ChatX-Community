(function attach(root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.KCHostPeerHandover = api;
})(typeof window !== 'undefined' ? window : globalThis, function createHostPeerHandover() {
  function connect(socket, timeoutMs) {
    return new Promise((resolve, reject) => {
      let settled = false;
      const finish = (error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        socket.removeListener?.('connect', onConnect);
        socket.removeListener?.('connect_error', onError);
        if (error) reject(error);
        else resolve();
      };
      const onConnect = () => finish();
      const onError = error => finish(error || new Error('Server sementara tidak dapat dihubungi'));
      const timer = setTimeout(() => finish(new Error('Koneksi server sementara timeout')), timeoutMs);
      socket.once('connect', onConnect);
      socket.once('connect_error', onError);
    });
  }

  function emitAck(socket, event, payload, timeoutMs) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`${event} timeout`)), timeoutMs);
      try {
        socket.emit(event, payload, result => {
          clearTimeout(timer);
          resolve(result || null);
        });
      } catch (error) {
        clearTimeout(timer);
        reject(error);
      }
    });
  }

  async function handoverTemporaryPeer(options = {}) {
    const { io, peer, hostUser, startHostPrimary, syncPeerToHost } = options;
    if (typeof io !== 'function' || typeof syncPeerToHost !== 'function' || typeof startHostPrimary !== 'function') {
      throw new Error('Koneksi atau fungsi sinkronisasi HOST tidak tersedia');
    }
    const primary = await startHostPrimary();
    if (!primary?.ok || !primary?.lanAddress) throw new Error(primary?.error || 'Listener HOST utama belum siap di LAN');

    const bridge = io(peer, {
      transports: ['websocket', 'polling'],
      reconnection: false,
      timeout: 6000,
      forceNew: true
    });
    try {
      await connect(bridge, 7000);
      const auth = await emitAck(bridge, 'hello', { ...hostUser, hostAuthority: true }, 8000);
      if (!auth?.ok || auth.peerTemporary !== true || !auth.handoverToken) {
        throw new Error(auth?.error || 'Peer bukan server sementara yang siap disinkronkan');
      }
      const synced = await syncPeerToHost({
        base: peer,
        token: auth.token,
        handoverToken: auth.handoverToken
      });
      if (!synced?.ok || synced.skipped) throw new Error(synced?.error || 'Sinkronisasi data server sementara gagal');

      const takeover = await emitAck(bridge, 'peer-handover-request', {
        syncedAt: Date.now(),
        handoverToken: auth.handoverToken,
        hostAddress: primary.lanAddress
      }, 10000);
      if (!takeover?.ok) throw new Error(takeover?.error || 'Server sementara belum mengonfirmasi handover');
      return { ok: true, synced, hostAddress: primary.lanAddress };
    } finally {
      try { bridge.removeAllListeners?.(); } catch {}
      try { bridge.disconnect?.(); } catch {}
    }
  }

  return { handoverTemporaryPeer };
});
