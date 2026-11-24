use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{io, path::{Path, PathBuf}, time::Duration};

pub(crate) const ELECTRON_HISTORY: &str = "chat-messages-v1.sqlite";

pub(crate) fn host_history_path(directory: &Path) -> io::Result<PathBuf> {
    let native = directory.join("chat.sqlite");
    if native.try_exists()? {
        return Ok(native);
    }
    let legacy = directory.join(ELECTRON_HISTORY);
    if legacy.try_exists()? {
        return Ok(legacy);
    }
    Ok(native)
}

pub(crate) fn initialize(path: &Path) -> rusqlite::Result<()> {
    Connection::open(path)?.execute_batch(
        "CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, ts INTEGER NOT NULL,
            payload_json TEXT NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_lab_chat_page ON chat_messages(conversation_id, ts, id);
        CREATE TABLE IF NOT EXISTS chat_read_markers (
            conversation_id TEXT NOT NULL, name TEXT NOT NULL, ts INTEGER NOT NULL,
            PRIMARY KEY(conversation_id, name)
        );",
    )
}

pub fn insert_message(path: &Path, item: &Value, timestamp: i64) -> rusqlite::Result<bool> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let count = db.execute(
        "INSERT INTO chat_messages(id, conversation_id, ts, payload_json, updated_at)
         VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
        rusqlite::params![item["id"].as_str(), item["conversationId"].as_str(), timestamp, item.to_string(), timestamp],
    )?;
    Ok(count == 1)
}

pub fn delete_conversation(path: &Path, id: &str) -> rusqlite::Result<()> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM chat_messages WHERE conversation_id=?1", [id])?;
    tx.execute("DELETE FROM chat_read_markers WHERE conversation_id=?1", [id])?;
    tx.commit()
}

pub fn write_batch(path: &Path, messages: &[Value], updated_at: i64) -> rusqlite::Result<()> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let transaction = db.transaction()?;
    stage_batch(&transaction, messages, updated_at)?;
    transaction.commit()
}

fn stage_batch(transaction: &rusqlite::Transaction<'_>, messages: &[Value], updated_at: i64) -> rusqlite::Result<()> {
        let mut statement = transaction.prepare(
            "INSERT INTO chat_messages(id, conversation_id, ts, payload_json, updated_at)
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET
             conversation_id=excluded.conversation_id, ts=excluded.ts,
             payload_json=excluded.payload_json, updated_at=excluded.updated_at",
        )?;
        for message in messages {
            let id = message["id"].as_str().filter(|id| !id.is_empty())
                .ok_or(rusqlite::Error::InvalidParameterName("id".into()))?;
            let conversation = message["conversationId"].as_str().filter(|id| !id.is_empty())
                .or_else(|| message["room"].as_str().filter(|id| !id.is_empty()))
                .unwrap_or("general");
            let timestamp = message["ts"].as_i64().filter(|ts| *ts != 0).unwrap_or(updated_at);
            statement.execute(rusqlite::params![id, conversation, timestamp, message.to_string(), updated_at])?;
        }
    Ok(())
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use std::{io::{BufRead, BufReader, Write}, process::{Command, Stdio}};

    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("KC_RUST_CRASH_TEST_DB") else { return };
        let mut db = Connection::open(path).unwrap();
        let transaction = db.transaction().unwrap();
        stage_batch(&transaction, &[serde_json::json!({"id":"durable","text":"uncommitted"})], 2).unwrap();
        println!("BATCH_STAGED");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(8));
        panic!("parent did not terminate crash fixture");
    }

    #[test]
    fn killed_writer_preserves_committed_history() {
        let directory = std::env::temp_dir().join(format!("kc-rust-crash-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("chat.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE chat_messages(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, ts INTEGER NOT NULL, payload_json TEXT NOT NULL, updated_at INTEGER NOT NULL);").unwrap();
        drop(db);
        let message = serde_json::json!({"id":"durable","text":"committed"});
        write_batch(&path, std::slice::from_ref(&message), 1).unwrap();

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "history::crash_tests::crash_child", "--nocapture"])
            .env("KC_RUST_CRASH_TEST_DB", &path).stdout(Stdio::piped()).spawn().unwrap();
        let staged = BufReader::new(child.stdout.take().unwrap()).lines()
            .any(|line| line.is_ok_and(|line| line.contains("BATCH_STAGED")));
        let killed = child.kill();
        let status = child.wait().unwrap();
        assert!(staged);
        killed.unwrap();
        assert!(!status.success());
        assert_eq!(read_snapshot(&path, 5000).unwrap(), vec![message]);
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)).unwrap(), "ok");
        drop(db);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
}

pub fn read_snapshot(path: &Path, limit: u32) -> rusqlite::Result<Vec<Value>> {
    let mut rows = read_rows(path, "SELECT payload_json FROM chat_messages ORDER BY ts DESC, rowid DESC LIMIT ?", [limit.max(1)])?;
    rows.reverse();
    Ok(rows)
}

pub fn storage_inventory(path:&Path)->rusqlite::Result<(u64,Vec<Value>)> {
    let db=open_reader(path)?;
    let transaction=db.unchecked_transaction()?;
    let count:i64=transaction.query_row("SELECT COUNT(*) FROM chat_messages",[],|row|row.get(0))?;
    let mut query=transaction.prepare("SELECT payload_json FROM chat_messages WHERE CASE WHEN json_valid(payload_json) THEN json_extract(payload_json,'$.file.stored') END IS NOT NULL ORDER BY ts DESC,id DESC")?;
    let mut rows=Vec::new();
    for raw in query.query_map([],|row|row.get::<_,String>(0))? {
        if let Ok(value)=serde_json::from_str::<Value>(&raw?) {rows.push(value);}
    }
    Ok((count as u64,rows))
}

pub fn latest(path: &Path, conversation: &str) -> rusqlite::Result<Option<Value>> {
    Ok(read_rows(path, "SELECT payload_json FROM chat_messages WHERE conversation_id=?1 ORDER BY ts DESC,id DESC LIMIT 1", [conversation])?.pop())
}

pub fn message(path: &Path, id: &str) -> rusqlite::Result<Option<Value>> {
    Ok(read_rows(path, "SELECT payload_json FROM chat_messages WHERE id=?1", [id])?.pop())
}

#[derive(Default)]
pub struct Search {
    pub text: String,
    pub kind: String,
    pub sender: String,
    pub tag: String,
    pub from: f64,
    pub to: f64,
}

impl Search {
    pub fn empty(&self) -> bool {
        self.text.is_empty() && self.sender.is_empty() && self.tag.is_empty() && self.from == 0.0 && self.to == 0.0
    }
}

pub fn search(path: &Path, conversation: &str, filter: &Search) -> rusqlite::Result<Vec<Value>> {
    let db = open_reader(path)?;
    let mut query = db.prepare("SELECT payload_json FROM chat_messages WHERE conversation_id=?1
        AND (?2=0 OR ts>=?2) AND (?3=0 OR ts<=?3) ORDER BY ts DESC,rowid DESC")?;
    let mut results = Vec::new();
    for raw in query.query_map(rusqlite::params![conversation, filter.from, filter.to], |row| row.get::<_, String>(0))? {
        let Ok(item) = serde_json::from_str::<Value>(&raw?) else { continue };
        let string = |value: &Value| value.as_str().unwrap_or_default().to_lowercase();
        let haystack = [&item["sender"], &item["text"], &item["file"]["name"], &item["reply"]["sender"],
            &item["reply"]["text"], &item["reply"]["fileName"], &item["workTag"]]
            .iter().map(|value| string(value)).collect::<Vec<_>>().join(" ");
        if !haystack.contains(&filter.text) || !string(&item["sender"]).contains(&filter.sender)
            || (!filter.tag.is_empty() && string(&item["workTag"]) != filter.tag) { continue }
        let has_file = item["file"].is_object();
        let matches_kind = match filter.kind.as_str() {
            "image" => has_file && string(&item["file"]["mimetype"]).starts_with("image/"),
            "file" => has_file,
            "psd" => has_file && string(&item["file"]["name"]).ends_with(".psd"),
            "text" => !has_file,
            "mention" => item["text"].as_str().unwrap_or_default().split('@').skip(1)
                .any(|tail| tail.bytes().next().is_some_and(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))),
            _ => true,
        };
        if !matches_kind { continue }
        let mut result = serde_json::json!({"id":item["id"],"ts":item["ts"],"sender":item["sender"],
            "text":item["text"].as_str().unwrap_or_default(),"workTag":item["workTag"].as_str().filter(|tag| !tag.is_empty()),"file":null});
        if has_file {
            let mut file = serde_json::Map::new();
            for key in ["name", "url", "mimetype"] {
                if let Some(value) = item["file"].get(key) { file.insert(key.into(), value.clone()); }
            }
            result["file"] = Value::Object(file);
        }
        results.push(result);
        if results.len() == 200 { break }
    }
    Ok(results)
}

pub fn attachment_conversations(path: &Path, stored: &str) -> rusqlite::Result<Vec<String>> {
    let db = open_reader(path)?;
    let mut statement = db.prepare(
        "SELECT DISTINCT conversation_id FROM chat_messages WHERE
         CASE WHEN json_valid(payload_json) THEN
             json_extract(payload_json,'$.file.stored')=?1 AND NOT COALESCE(json_extract(payload_json,'$.deleted'),0)
         ELSE 0 END",
    )?;
    let rows = statement.query_map([stored], |row| row.get(0))?;
    rows.collect()
}

pub fn gallery_attachments(path: &Path, conversations: &Value) -> rusqlite::Result<Vec<Value>> {
    read_rows(path, "SELECT payload_json FROM chat_messages
        WHERE conversation_id IN (SELECT key FROM json_each(?1))
        AND CASE WHEN json_valid(payload_json) THEN
            json_extract(payload_json,'$.file.stored') IS NOT NULL
            AND NOT COALESCE(json_extract(payload_json,'$.deleted'),0)
            AND NOT COALESCE(json_extract(payload_json,'$.attachmentRemoved'),0)
        ELSE 0 END ORDER BY ts DESC,id DESC", [conversations.to_string()])
}

pub fn update_receipt(
    path: &Path, id: &str, conversation: &str, recipient: &str, read: bool, now: i64,
) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension;
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx.query_row(
        "SELECT payload_json FROM chat_messages WHERE id=?1 AND conversation_id=?2",
        [id, conversation], |row| row.get(0),
    ).optional()?;
    let Some(mut item) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
        return Ok(None);
    };
    let Some(sender) = item["sender"].as_str() else {
        return Ok(None);
    };
    if sender.trim().to_lowercase() == recipient.trim().to_lowercase() {
        return Ok(None);
    }
    if set_dm_receipt(&mut item, read, now) {
        stage_batch(&tx, std::slice::from_ref(&item), now)?;
    }
    tx.commit()?;
    Ok(Some(item))
}

fn set_dm_receipt(item: &mut Value, read: bool, now: i64) -> bool {
    let previous = &item["receipt"];
    let delivered = previous["deliveredAt"].as_i64().filter(|ts| *ts > 0).unwrap_or(now);
    let seen = previous["readAt"].as_i64().filter(|ts| *ts > 0).unwrap_or(if read { now } else { 0 });
    let receipt = serde_json::json!({"deliveredAt": delivered, "readAt": seen});
    let changed = previous != &receipt;
    item["receipt"] = receipt;
    changed
}

pub enum ReactionUpdate {
    Applied(Value),
    Missing,
    Full,
}

pub enum MessageChange<'a> {
    Edit { text: &'a str, actor: &'a str, is_host: bool },
    Delete { actor: &'a str },
}

pub fn change_message(
    path: &Path, id: &str, conversation: &str, change: MessageChange<'_>, now: i64,
) -> rusqlite::Result<Result<Value, &'static str>> {
    use rusqlite::OptionalExtension;
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx.query_row(
        "SELECT payload_json FROM chat_messages WHERE id=?1 AND conversation_id=?2",
        [id, conversation], |row| row.get(0),
    ).optional()?;
    let Some(mut item) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()).filter(Value::is_object) else {
        return Ok(Err("Pesan tidak ditemukan"));
    };
    match change {
        MessageChange::Edit { text, actor, is_host } => {
            if item["deleted"] == true {
                return Ok(Err("Pesan sudah dihapus"));
            }
            if !is_host && !item["sender"].as_str().is_some_and(|sender| {
                sender.trim().to_lowercase() == actor.trim().to_lowercase()
            }) {
                return Ok(Err("Kamu hanya bisa mengedit pesan sendiri"));
            }
            item["text"] = serde_json::json!(text);
            item["editedAt"] = serde_json::json!(now);
        }
        MessageChange::Delete { actor } => {
            item["deleted"] = serde_json::json!(true);
            item["deletedAt"] = serde_json::json!(now);
            item["deletedBy"] = serde_json::json!(actor);
            item["text"] = serde_json::json!("");
            item["file"] = Value::Null;
        }
    }
    stage_batch(&tx, std::slice::from_ref(&item), now)?;
    tx.commit()?;
    Ok(Ok(item))
}

pub fn toggle_reaction(
    path: &Path, id: &str, conversation: &str, name: &str, emoji: &str, now: i64,
) -> rusqlite::Result<ReactionUpdate> {
    use rusqlite::OptionalExtension;
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let raw: Option<String> = tx.query_row(
        "SELECT payload_json FROM chat_messages WHERE id=?1 AND conversation_id=?2",
        [id, conversation], |row| row.get(0),
    ).optional()?;
    let Some(mut item) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()).filter(Value::is_object) else {
        return Ok(ReactionUpdate::Missing);
    };
    if item["deleted"] == true {
        return Ok(ReactionUpdate::Missing);
    }
    let key = name.trim().to_lowercase();
    let mut reactions = item["reactions"].as_array().cloned().unwrap_or_default();
    let same_user = |entry: &Value| entry["name"].as_str().is_some_and(|name| name.trim().to_lowercase() == key);
    let previous = reactions.iter().find(|entry| same_user(entry)).and_then(|entry| entry["emoji"].as_str());
    let remove = previous == Some(emoji);
    reactions.retain(|entry| !same_user(entry));
    if !remove {
        if reactions.len() >= 1000 {
            return Ok(ReactionUpdate::Full);
        }
        reactions.push(serde_json::json!({"name":name,"emoji":emoji,"at":now}));
    }
    item["reactions"] = Value::Array(reactions);
    stage_batch(&tx, std::slice::from_ref(&item), now)?;
    tx.commit()?;
    Ok(ReactionUpdate::Applied(item))
}

pub fn read_marker(path: &Path, conversation: &str, name: &str) -> rusqlite::Result<f64> {
    open_reader(path)?.query_row(
        "SELECT COALESCE((SELECT ts FROM chat_read_markers WHERE conversation_id=?1 AND name=?2),0)",
        rusqlite::params![conversation, name.trim().to_lowercase()], |row| row.get(0),
    )
}

pub fn mark_read(path: &Path, conversation: &str, name: &str, dm: bool, now: i64) -> rusqlite::Result<Vec<Value>> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.pragma_update(None, "synchronous", "FULL")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let key = name.trim().to_lowercase();
    let profile = crate::profiles::key(name);
    let mut changed = Vec::new();
    {
        let mut query = tx.prepare("SELECT payload_json FROM chat_messages WHERE conversation_id=?1 ORDER BY ts DESC,id DESC LIMIT 200")?;
        let rows = query.query_map([conversation], |row| row.get::<_, String>(0))?;
        for row in rows {
            let Ok(mut item) = serde_json::from_str::<Value>(&row?) else { continue };
            if item["sender"].as_str().is_none_or(|sender| sender.trim().to_lowercase() == key) { continue }
            if dm {
                if !set_dm_receipt(&mut item, true, now) { continue }
            } else {
                if !item["readBy"].is_object() { item["readBy"] = serde_json::json!({}); }
                let readers = item["readBy"].as_object_mut().expect("readBy object");
                if readers.contains_key(&profile) { continue }
                readers.insert(profile.clone(), serde_json::json!({"name":name,"at":now}));
            }
            changed.push(item);
        }
    }
    stage_batch(&tx, &changed, now)?;
    tx.execute(
        "INSERT INTO chat_read_markers(conversation_id, name, ts)
         VALUES (?1, ?2, COALESCE((SELECT MAX(ts) FROM chat_messages WHERE conversation_id=?1), 0))
         ON CONFLICT(conversation_id,name) DO UPDATE SET ts=MAX(ts,excluded.ts)",
        rusqlite::params![conversation, key],
    )?;
    tx.commit()?;
    Ok(changed)
}

pub fn page(path: &Path, conversation: &str, limit: i64, before_ts: Option<f64>, before_id: &str) -> rusqlite::Result<Value> {
    let limit = if limit == 0 { 120 } else { limit.clamp(20, 200) };
    let mut items = match before_ts {
        None => read_rows(path, "SELECT payload_json FROM chat_messages WHERE conversation_id=?1 ORDER BY ts DESC,id DESC LIMIT ?2",
            rusqlite::params![conversation, limit + 1]),
        Some(ts) if before_id.is_empty() => read_rows(path, "SELECT payload_json FROM chat_messages WHERE conversation_id=?1 AND ts<?2 ORDER BY ts DESC,id DESC LIMIT ?3",
            rusqlite::params![conversation, ts, limit + 1]),
        Some(ts) => read_rows(path, "SELECT payload_json FROM chat_messages WHERE conversation_id=?1 AND (ts,id)<(?2,?3) ORDER BY ts DESC,id DESC LIMIT ?4",
            rusqlite::params![conversation, ts, before_id, limit + 1]),
    }?;
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    items.reverse();
    let cursor = items.first().map(|item| {
        let mut cursor = serde_json::Map::new();
        for field in ["ts", "id"] {
            if let Some(value) = item.get(field) {
                cursor.insert(field.into(), value.clone());
            }
        }
        cursor
    });
    Ok(serde_json::json!({"items":items,"hasMore":has_more,"cursor":cursor}))
}

pub fn legacy_general(path: &Path) -> rusqlite::Result<Vec<Value>> {
    let mut items = read_rows(path, "SELECT payload_json FROM chat_messages WHERE conversation_id='general' ORDER BY ts DESC,id DESC LIMIT 500", [])?;
    items.reverse();
    Ok(items)
}

pub fn around(path: &Path, conversation: &str, anchor: &str) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension;
    let db = open_reader(path)?;
    let snapshot = db.unchecked_transaction()?;
    let stamp = snapshot.query_row(
        "SELECT ts FROM chat_messages WHERE conversation_id=?1 AND id=?2",
        [conversation, anchor], |row| row.get::<_, f64>(0),
    ).optional()?;
    let Some(stamp) = stamp else { return Ok(None) };
    let fetch = |sql: &str| -> rusqlite::Result<Vec<Value>> {
        let mut query = snapshot.prepare(sql)?;
        let rows = query.query_map(rusqlite::params![conversation, stamp, anchor], |row| row.get::<_, String>(0))?;
        let mut items = Vec::new();
        for row in rows {
            if let Ok(value) = serde_json::from_str::<Value>(&row?) {
                items.push(value);
            }
        }
        Ok(items)
    };
    let mut before = fetch("SELECT payload_json FROM chat_messages WHERE conversation_id=?1 AND (ts,id)<=(?2,?3) ORDER BY ts DESC,id DESC LIMIT 61")?;
    let mut after = fetch("SELECT payload_json FROM chat_messages WHERE conversation_id=?1 AND (ts,id)>(?2,?3) ORDER BY ts,id LIMIT 61")?;
    let has_more = before.len() > 60;
    let has_newer = after.len() > 60;
    before.truncate(60);
    before.reverse();
    after.truncate(60);
    before.extend(after);
    let cursor = before.first().map(|row| serde_json::json!({"ts":row["ts"],"id":row["id"]}));
    Ok(Some(serde_json::json!({"items":before,"hasMore":has_more,"hasNewer":has_newer,"cursor":cursor})))
}

fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_secs(3))?;
    Ok(db)
}

pub fn unread(path: &Path, conversation: &str, name: &str, since: f64) -> rusqlite::Result<i64> {
    open_reader(path)?.query_row(
        "SELECT COUNT(*) FROM chat_messages WHERE conversation_id=? AND ts>?
         AND lower(trim(json_extract(payload_json,'$.sender')))<>?",
        rusqlite::params![conversation, since, name.trim_matches(|c: char|
            c == '\u{feff}' || (c.is_whitespace() && c != '\u{0085}')).to_lowercase()],
        |row| row.get(0),
    )
}

fn read_rows(path: &Path, sql: &str, params: impl rusqlite::Params) -> rusqlite::Result<Vec<Value>> {
    let db = open_reader(path)?;
    let mut query = db.prepare(sql)?;
    let mut rows = Vec::new();
    for raw in query.query_map(params, |row| row.get::<_, String>(0))? {
        if let Ok(value) = serde_json::from_str::<Value>(&raw?) {
            let keep = match &value {
                Value::Null => false,
                Value::Bool(value) => *value,
                Value::Number(value) => value.as_f64() != Some(0.0),
                Value::String(value) => !value.is_empty(),
                _ => true,
            };
            if keep {
                rows.push(value);
            }
        }
    }
    Ok(rows)
}

pub(crate) fn import_read_markers(data: &Path, history: &Path) -> io::Result<usize> {
    let read_json = |name: &str| -> io::Result<Value> {
        match std::fs::File::open(data.join(name)) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Value::Null),
            Err(error) => Err(error),
        }
    };
    let legacy = read_json("conversation-state.json")?;
    let Some(states) = legacy.as_object() else { return Ok(0) };
    let registry = read_json("approved-devices.json")?;
    let moderation = read_json("moderation.json")?;
    let conversations = read_json("conversations.json")?;
    let mut names = std::collections::HashSet::new();
    for roster in [&registry, &moderation["knownUsers"]] {
        for row in roster.as_object().into_iter().flat_map(|records| records.values()) {
            if let Some(name) = row["name"].as_str().map(str::trim).filter(|name| !name.is_empty()) {
                names.insert(name.to_lowercase());
            }
        }
    }
    for conversation in conversations.as_array().into_iter().flatten() {
        for name in conversation["members"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !name.trim().is_empty() {
                names.insert(name.trim().to_lowercase());
            }
        }
    }
    initialize(history).map_err(io::Error::other)?;
    let mut db = Connection::open(history).map_err(io::Error::other)?;
    let transaction = db.transaction().map_err(io::Error::other)?;
    let mut count = 0;
    {
        let mut write = transaction.prepare(
            "INSERT INTO chat_read_markers(conversation_id,name,ts) VALUES(?1,?2,?3)
             ON CONFLICT(conversation_id,name) DO UPDATE SET ts=excluded.ts WHERE excluded.ts > chat_read_markers.ts",
        ).map_err(io::Error::other)?;
        for name in names {
            // Electron replaces each run of invalid characters independently,
            // including a run immediately beside an existing hyphen.
            let profile = name.split(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '_' | '-'))
                .filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
            let profile = profile.trim_matches('-');
            let key = if profile.is_empty() { "editor" } else { profile };
            let Some(read) = states.get(key).and_then(Value::as_object) else { continue };
            for (conversation, timestamp) in read {
                if let Some(stamp) = timestamp.as_i64().filter(|stamp| *stamp > 0) {
                    if !conversation.is_empty() {
                        count += write.execute(rusqlite::params![conversation, name, stamp]).map_err(io::Error::other)?;
                    }
                }
            }
        }
    }
    transaction.commit().map_err(io::Error::other)?;
    Ok(count)
}
