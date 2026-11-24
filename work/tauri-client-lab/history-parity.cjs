'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { DatabaseSync } = require('node:sqlite');
const { execFileSync, spawnSync } = require('node:child_process');

const name = 'chat-messages-v1.sqlite';
const source = fs.readdirSync(__dirname)
    .filter(entry => entry.startsWith('chat-qa-'))
    .map(entry => path.join(__dirname, entry, name))
    .filter(file => fs.existsSync(file))
    .find(file => {
        const db = new DatabaseSync(file, { readOnly: true });
        try { return db.prepare('SELECT COUNT(*) AS count FROM chat_messages').get().count > 0; }
        finally { db.close(); }
    });
assert(source, 'Run the isolated chat test to generate a HOST database first');
const directory = fs.mkdtempSync(path.join(__dirname, 'history-qa-'));
const file = path.join(directory, name);
fs.copyFileSync(source, file);
const db = new DatabaseSync(file);
try {
    db.exec('DROP INDEX IF EXISTS idx_chat_attachment');
    const insert = db.prepare('INSERT INTO chat_messages VALUES (?, ?, ?, ?, ?)');
    for (const [id, payload] of [['a', '{"text":"Pesan uji 🧪"}'], ['b', '{"file":{"name":"cetak.png"}}'], ['bad', '{']]) {
        insert.run('history-qa-' + id, 'general', 1, payload, 1);
    }
} finally { db.close(); }

const reference = new DatabaseSync(file, { readOnly: true });
let expected;
try {
    expected = reference.prepare('SELECT payload_json FROM chat_messages ORDER BY ts DESC,rowid DESC LIMIT 5000')
        .all().reverse().flatMap(row => {
            try { return [JSON.parse(row.payload_json)].filter(Boolean); }
            catch { return []; }
        });
} finally { reference.close(); }
const hash = () => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const before = hash();
const binary = process.env.KC_LAB_EXE || path.join(__dirname, 'src-tauri/target/debug/chatx-client-lab.exe');
const output = execFileSync(binary,
    ['--verify-history', file], { encoding: 'utf8', windowsHide: true, timeout: 10000 });
assert.deepEqual(JSON.parse(output), expected);
assert.equal(hash(), before);
console.log('PASS: Rust matches legacy SQLite snapshot, including ties and malformed rows; database bytes unchanged');

const batch = path.join(directory, 'batch.json');
const message = { id: 'history-qa-a', conversationId: 'group-qa', ts: 42, text: 'Edited', file: { name: 'a.png' } };
const run = items => {
    fs.writeFileSync(batch, JSON.stringify(items));
    return spawnSync(binary, ['--verify-history-write', file, batch], { encoding: 'utf8', windowsHide: true, timeout: 10000 });
};
const saved = run([message, { id: 'history-qa-new', room: 'group-qa', ts: 43 }]);
assert.equal(saved.status, 0, saved.stderr);
const readRows = () => {
    const db = new DatabaseSync(file, { readOnly: true });
    try { return db.prepare('SELECT * FROM chat_messages ORDER BY rowid').all(); }
    finally { db.close(); }
};
const committed = readRows();
const edited = committed.find(row => row.id === message.id);
assert.deepEqual(JSON.parse(edited.payload_json), message);
assert.equal(edited.conversation_id, 'group-qa');
assert.equal(edited.ts, 42);
assert.equal(committed.find(row => row.id === 'history-qa-new').conversation_id, 'group-qa');
const failed = run([{ ...message, text: 'Uncommitted' }, { ts: 44 }]);
assert.equal(failed.status, 1, failed.stderr);
assert.deepEqual(readRows(), committed);
console.log('PASS: legacy SQLite reads Rust inserts and updates; a failed batch rolls back');

const lockedMessage = { id: 'history-qa-locked', conversationId: 'general', ts: 45, text: 'Retry after lock' };
const holder = new DatabaseSync(file);
try {
    holder.exec('BEGIN IMMEDIATE');
    const locked = run([lockedMessage]);
    assert.ifError(locked.error);
    assert.equal(locked.status, 1, locked.stderr);
    assert.match(locked.stderr, /database is locked/i);
} finally {
    holder.exec('ROLLBACK');
    holder.close();
}
assert.deepEqual(readRows(), committed);
for (let retry = 0; retry < 2; retry++) {
    const result = run([lockedMessage]);
    assert.equal(result.status, 0, result.stderr);
}
const retried = readRows().filter(row => row.id === lockedMessage.id);
assert.equal(retried.length, 1);
assert.deepEqual(JSON.parse(retried[0].payload_json), lockedMessage);
console.log('PASS: locked writer times out without changes; repeated retry stores one message');

const paging = new DatabaseSync(file);
try {
    const insert = paging.prepare('INSERT INTO chat_messages VALUES (?, ?, ?, ?, ?)');
    for (let index = 0; index < 57; index++) {
        const id = 'page-' + String(index).padStart(3, '0');
        insert.run(id, 'paging', 100, JSON.stringify({ id, ts: 100 }), 100);
    }
    const { createChatQueries } = require('../source-v3.7.11/ChatX/lib/chat-queries');
    const legacy = createChatQueries(() => paging, () => []);
    const ids = [];
    let cursor = null;
    do {
        const request = { conversationId: 'paging', limit: 20, beforeTs: cursor?.ts, beforeId: cursor?.id };
        const page = JSON.parse(execFileSync(binary, ['--verify-history-page', file, JSON.stringify(request)],
            { encoding: 'utf8', windowsHide: true, timeout: 10000 }));
        assert.deepEqual(page, legacy.page(request.conversationId, request.limit, request.beforeTs, request.beforeId));
        ids.push(...page.items.map(item => item.id));
        cursor = page.hasMore ? page.cursor : null;
    } while (cursor && ids.length <= 57);
    assert.equal(ids.length, 57);
    assert.equal(new Set(ids).size, 57);
    console.log('PASS: legacy and Rust pagination match across 57 messages sharing a timestamp');
    const fixtures = [
        ['own', 'read-state', 101, { sender: ' ALICE ' }],
        ['other', 'read-state', 101, { sender: 'Bob' }],
        ['old', 'read-state', 100, { sender: 'Bob' }],
        ['elsewhere', 'other-conversation', 101, { sender: 'Bob' }],
        ['missing', 'read-state', 101, {}],
    ];
    for (const [id, conversation, ts, payload] of fixtures) {
        insert.run('unread-' + id, conversation, ts, JSON.stringify(payload), ts);
    }
    const request = { conversationId: 'read-state', name: ' Alice ', since: 100 };
    const count = Number(execFileSync(binary, ['--verify-history-unread', file, JSON.stringify(request)],
        { encoding: 'utf8', windowsHide: true, timeout: 10000 }).trim());
    assert.equal(count, legacy.unread(request.conversationId, request.name, request.since));
    assert.equal(count, 1);
    console.log('PASS: unread count matches legacy across own messages, cutoff, other rooms, and missing sender');
} finally { paging.close(); }
