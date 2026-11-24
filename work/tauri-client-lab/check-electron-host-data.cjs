'use strict';

const fs = require('node:fs');
const path = require('node:path');
const {DatabaseSync} = require('node:sqlite');

const profile = process.argv[2] || path.join(process.env.APPDATA, 'ChatX Chat');
const config = JSON.parse(fs.readFileSync(path.join(profile, 'config.json'), 'utf8').replace(/^\uFEFF/, ''));
if (config.role !== 'server') throw Error('Profil yang dipilih bukan HOST Electron.');

const serverData = path.join(profile, 'server-data');
const results = {};
for (const [name, file, table] of [
    ['chat', 'chat-messages-v1.sqlite', 'chat_messages'],
    ['gallery', 'gallery-index-v2.sqlite', null],
]) {
    const database = new DatabaseSync(path.join(serverData, file), {readOnly: true});
    try {
        database.exec('PRAGMA busy_timeout=3000; PRAGMA query_only=ON; BEGIN');
        const integrity = database.prepare('PRAGMA quick_check(1)').get().quick_check;
        if (integrity !== 'ok') throw Error('Pemeriksaan integritas gagal untuk ' + name);
        const count = table ? database.prepare('SELECT COUNT(*) AS count FROM chat_messages').get().count : undefined;
        results[name] = {integrity, count};
    } finally {
        database.close();
    }
}
console.log(JSON.stringify(results));
