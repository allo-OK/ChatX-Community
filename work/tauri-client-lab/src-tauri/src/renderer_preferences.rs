use serde_json::Value;
use std::{fs::File, io::{self, Read}, path::Path};

const MAX_EXPORT: u64 = 5 * 1024 * 1024;
const PREFIXES: &[&str] = &[
    "kc-theme:", "kc-appearance:", "kc-conversation-prefs:",
    "kc-hidden-messages:", "kc-chat-scroll:", "kc-share-activity:",
];

pub(crate) fn initialization_script(directory: &Path) -> io::Result<Option<String>> {
    let file = match File::open(directory.join("renderer-preferences.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_EXPORT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EXPORT {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Ekspor preferensi terlalu besar"));
    }
    let mut export: Value = serde_json::from_slice(&bytes)?;
    if export["format"].as_u64() != Some(1) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Format ekspor preferensi tidak valid"));
    }
    let entries: Vec<(String, String)> = serde_json::from_value(export["entries"].take())?;
    if entries.iter().any(|(key, _)| !allowed(key)) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Kunci ekspor preferensi tidak valid"));
    }
    let entries = serde_json::to_string(&entries)?;
    Ok(Some(format!(r#"(() => {{
        const marker = 'kc-rust-electron-preferences-v1';
        if (localStorage.getItem(marker) !== null) return;
        for (const [key, value] of {entries}) {{
            if (localStorage.getItem(key) === null) localStorage.setItem(key, value);
        }}
        localStorage.setItem(marker, '1');
    }})();"#)))
}

fn allowed(key: &str) -> bool {
    matches!(key, "kc.slither.skin" | "kc.gallery.searchScope")
        || PREFIXES.iter().any(|prefix| key.strip_prefix(prefix).is_some_and(|name| !name.is_empty()))
}
