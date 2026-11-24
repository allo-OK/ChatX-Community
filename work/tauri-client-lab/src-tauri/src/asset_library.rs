use serde_json::{json, Map, Value};
use std::{io, path::{Path,PathBuf}, sync::Mutex};

pub struct Library { path: PathBuf, rows: Mutex<Map<String,Value>> }

pub fn kind(file: &Value) -> Option<&'static str> {
    match Path::new(file["name"].as_str()?).extension()?.to_str()?.to_lowercase().as_str() {
        "ttf" | "otf" => Some("fonts"), "psd" => Some("psds"), _ => None,
    }
}

impl Library {
    pub fn open(path: PathBuf, history: &Path) -> io::Result<Self> {
        let mut rows: Map<String,Value> = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error),
        };
        for kind in ["fonts","psds"] {
            if !rows.get(kind).is_some_and(Value::is_array) {rows.insert(kind.into(),json!([]));}
        }
        let library = Self {path,rows:Mutex::new(rows)};
        // The legacy server rebuilds these entries from its last 5,000 loaded messages at startup.
        let history = crate::history::read_snapshot(history,5000).map_err(io::Error::other)?;
        {
            let mut rows = library.rows.lock().expect("asset library");
            let mut next = rows.clone();
            for message in history {update(&mut next,&message);}
            if next != *rows {
                crate::host::atomic_save(&library.path,&next)?;
                *rows = next;
            }
        }
        Ok(library)
    }

    pub fn register(&self, message: &Value) -> Result<Option<&'static str>,String> {
        let Some(kind) = kind(&message["file"]) else {return Ok(None)};
        let mut rows = self.rows.lock().expect("asset library");
        let mut next = rows.clone();
        update(&mut next,message);
        if next != *rows {
            crate::host::atomic_save(&self.path,&next).map_err(|error|format!("Pustaka font/PSD belum tersimpan: {error}"))?;
            *rows = next;
        }
        Ok(Some(kind))
    }

    pub fn list(&self, kind: &str) -> Result<Value,String> {
        if !["fonts","psds"].contains(&kind) {return Err("Library tidak valid".into())}
        let rows = self.rows.lock().expect("asset library");
        Ok(json!({"ok":true,"kind":kind,"items":rows[kind].as_array().expect("asset rows").iter().take(1000).collect::<Vec<_>>()}))
    }
}

fn update(rows: &mut Map<String,Value>, message: &Value) {
    let file = &message["file"];
    let Some(kind) = kind(file) else {return};
    let text = |field: &str| file[field].as_str().unwrap_or_default().to_owned();
    let hash = text("hash");
    let stored = text("stored");
    let key = if !hash.is_empty() {hash.clone()} else if !stored.is_empty() {stored.clone()}
        else {format!("{}|{}",text("name"),file["size"].as_u64().unwrap_or_default())};
    let row = json!({"key":key,"name":text("name"),"stored":stored,"hash":hash,"size":file["size"].as_u64().unwrap_or_default(),
        "mimetype":text("mimetype"),"url":text("url"),"downloadUrl":text("downloadUrl"),"sender":message["sender"].as_str().unwrap_or_default(),
        "messageId":message["id"].as_str().unwrap_or_default(),"ts":message["ts"].as_u64().filter(|ts|*ts>0).unwrap_or_else(||std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64)});
    let list = rows[kind].as_array_mut().expect("asset rows");
    if let Some(existing) = list.iter_mut().find(|entry|entry["key"] == key) {
        if let Some(existing) = existing.as_object_mut() {existing.extend(row.as_object().unwrap().clone());}
        else { *existing = row; }
    } else {list.insert(0,row);}
    list.truncate(2000);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_legacy_metadata_and_caps_results_without_resetting_corrupt_data() {
        let directory = std::env::temp_dir().join(format!("kc-asset-library-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let db_path = directory.join("chat.sqlite");
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch("CREATE TABLE chat_messages(id TEXT PRIMARY KEY,conversation_id TEXT,ts INTEGER,payload_json TEXT,updated_at INTEGER);").unwrap();
        drop(db);
        let path = directory.join("asset-library.json");
        let fonts:Vec<_> = (0..2001).map(|i|json!({"key":format!("legacy-{i}"),"name":format!("Font-{i}.ttf"),"custom":"legacy"})).collect();
        crate::host::atomic_save(&path,json!({"fonts":fonts,"psds":[]}).as_object().unwrap()).unwrap();
        let library = Library::open(path.clone(),&db_path).unwrap();
        assert_eq!(library.list("fonts").unwrap()["items"].as_array().unwrap().len(),1000);
        let mut message = json!({"id":"m1","ts":100,"sender":"Editor","file":{"name":"New.TTF","hash":"new-hash","size":10}});
        library.register(&message).unwrap();
        message["id"] = json!("m2");
        library.register(&message).unwrap();
        let saved:Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["fonts"].as_array().unwrap().len(),2000);
        assert_eq!(saved["fonts"][0]["messageId"],"m2");
        assert_eq!(saved["fonts"][1]["custom"],"legacy");
        drop(library);
        std::fs::write(&path,b"corrupt original").unwrap();
        assert!(Library::open(path.clone(),&db_path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(),b"corrupt original");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
