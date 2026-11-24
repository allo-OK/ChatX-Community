'use strict';

/** Move SQLite chat rows from duplicate conversation IDs to their canonical ID. */
function remapChatDatabaseConversations(db, remap) {
  if (!db || !(remap instanceof Map) || remap.size === 0) return 0;
  const rows = db.prepare('SELECT id,conversation_id,payload_json FROM chat_messages').all();
  const update = db.prepare('UPDATE chat_messages SET conversation_id=?,payload_json=?,updated_at=? WHERE id=?');
  let changed = 0;
  db.exec('BEGIN');
  try {
    for (const row of rows) {
      let item;
      try { item = JSON.parse(String(row.payload_json || '')); } catch { continue; }
      const prior = String(row.conversation_id || item.conversationId || item.room || '');
      const target = remap.get(prior) || remap.get(String(item.conversationId || '')) || remap.get(String(item.room || ''));
      if (!target) continue;
      item.conversationId = target;
      item.room = target;
      update.run(String(target), JSON.stringify(item), Date.now(), String(row.id));
      changed++;
    }
    db.exec('COMMIT');
    return changed;
  } catch (error) {
    try { db.exec('ROLLBACK'); } catch (_) {}
    throw error;
  }
}

module.exports = { remapChatDatabaseConversations };
