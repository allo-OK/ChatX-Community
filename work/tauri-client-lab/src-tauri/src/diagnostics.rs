use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const RESOLVED_RETENTION_MS: u64 = 14 * 24 * 60 * 60 * 1000;

pub struct Diagnostics {
    path: PathBuf,
    data: Mutex<Map<String, Value>>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn clip(text: &str, limit: usize) -> String {
    let mut line_break = false;
    text.chars()
        .filter_map(|character| {
            let previous = std::mem::replace(&mut line_break, matches!(character, '\r' | '\n'));
            if line_break {
                (!previous).then_some(' ')
            } else {
                Some(character)
            }
        })
        .take(limit)
        .collect()
}

fn field(raw: &Value, key: &str, default: &str, limit: usize) -> String {
    let value = value_text(&raw[key]);
    clip(if value.is_empty() { default } else { &value }, limit)
}

impl Diagnostics {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let mut data: Map<String, Value> = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error),
        };
        let groups = data
            .entry("groups")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::new(
                io::ErrorKind::InvalidData,
                "Format grup diagnostik tidak valid.",
            ))?;
        let previous_count = groups.len();
        let cutoff = now().saturating_sub(RESOLVED_RETENTION_MS);
        groups.retain(|_, group| {
            let resolved_at = group["statusChangedAt"].as_u64().filter(|ts| *ts > 0)
                .or_else(|| group["lastSeen"].as_u64()).unwrap_or_default();
            group["status"] != "resolved" || resolved_at >= cutoff
        });
        if groups.len() != previous_count {
            crate::host::atomic_save(&path, &data)?;
        }
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    pub fn record(&self, raw: &Value, identity: &Value) -> io::Result<Value> {
        let role = match field(raw, "role", "", 20).to_lowercase().as_str() {
            "host" => "host",
            "client" => "client",
            _ if identity["isHost"] == true => "host",
            _ => "client",
        };
        let kind = field(raw, "type", "unknown", 120);
        let source = field(raw, "source", "renderer", 80);
        let severity = raw["severity"]
            .as_str()
            .filter(|s| ["info", "warning", "error", "fatal"].contains(s))
            .unwrap_or("error");
        let detail = field(
            &raw["detail"],
            "message",
            raw["detail"].as_str().unwrap_or("Unknown error"),
            1200,
        );
        let message = field(raw, "message", &detail, 1200);
        let stack = field(
            raw,
            "stack",
            raw["detail"]["stack"].as_str().unwrap_or_default(),
            8000,
        );
        let version = field(raw, "version", "", 40);
        let pc = field(raw, "pc", identity["pc"].as_str().unwrap_or_default(), 100);
        let user = field(
            raw,
            "user",
            identity["name"].as_str().unwrap_or_default(),
            80,
        );
        let device = field(
            raw,
            "deviceId",
            identity["deviceId"].as_str().unwrap_or_default(),
            140,
        );
        let mut context = Map::new();
        if let Some(values) = raw["context"].as_object() {
            for (key, value) in values.iter().take(30) {
                let lower = key.to_ascii_lowercase();
                if [
                    "text",
                    "messagebody",
                    "chatcontent",
                    "filecontent",
                    "base64",
                ]
                .iter()
                .any(|word| lower.contains(word))
                {
                    continue;
                }
                context.insert(clip(key, 80), json!(clip(&value_text(value), 800)));
            }
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(
                format!("{role}::{kind}::{source}::{}::{stack}", clip(&message, 500)).as_bytes()
            )
        );
        let signature = &hash[..24];
        let ts = now();
        let occurrence = json!({"id":uuid::Uuid::new_v4().to_string(),"ts":ts,"version":version,"pc":pc,"user":user,
            "deviceId":device,"message":message,"stack":stack,"context":context});
        let queue_id = raw["queueId"].as_str()
            .filter(|_| identity["isHost"] == true)
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .map(|id| id.to_string());
        let mut data = self.data.lock().expect("diagnostics store");
        if let Some(receipt) = queue_id.as_ref().and_then(|id| {
            data.get("hostQueueReceipts").and_then(Value::as_array)?.iter()
                .find(|row| row["id"].as_str() == Some(id.as_str()))
        }) {
            let mut reply = receipt["reply"].clone();
            reply["replayed"] = json!(true);
            return Ok(reply);
        }
        let mut next = data.clone();
        let groups = next
            .get_mut("groups")
            .and_then(Value::as_object_mut)
            .expect("validated diagnostics groups");
        let group = groups.entry(signature).or_insert_with(|| {
            json!({"signature":signature,"firstSeen":ts,"status":"open",
            "role":role,"type":kind,"source":source,"deviceId":device,"occurrences":[]})
        });
        group["lastSeen"] = json!(ts);
        group["count"] = json!(group["count"].as_u64().unwrap_or(0).saturating_add(1));
        for (key, value) in [
            ("severity", severity),
            ("message", &message),
            ("stack", &stack),
        ] {
            group[key] = json!(value);
        }
        for (key, value) in [("version", &version), ("pc", &pc), ("user", &user)] {
            if !value.is_empty() || group.get(key).is_none() {
                group[key] = json!(value);
            }
        }
        if group["status"] == "resolved" {
            group["status"] = json!("regressed");
        }
        let mut occurrences = group["occurrences"].as_array().cloned().unwrap_or_default();
        if occurrences.len() >= 20 {
            occurrences.drain(..occurrences.len() - 19);
        }
        occurrences.push(occurrence);
        group["occurrences"] = json!(occurrences);
        let reply = json!({"ok":true,"signature":signature,"count":group["count"],"status":group["status"]});
        if groups.len() > 500 {
            let mut keys: Vec<_> = groups
                .iter()
                .map(|(key, row)| (key.clone(), row["lastSeen"].as_u64().unwrap_or(0)))
                .collect();
            keys.sort_by_key(|(key, seen)| (key != signature, std::cmp::Reverse(*seen)));
            for (key, _) in keys.into_iter().skip(500) {
                groups.remove(&key);
            }
        }
        let count = next
            .get("totalReports")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        next.insert("totalReports".into(), json!(count.saturating_add(1)));
        next.insert("lastReportAt".into(), json!(ts));
        if let Some(id) = queue_id {
            let stored = next.entry("hostQueueReceipts").or_insert_with(|| json!([]));
            let mut receipts = stored.as_array_mut().map(std::mem::take).unwrap_or_default();
            if receipts.len() >= 1000 {
                receipts.drain(..receipts.len() - 999);
            }
            receipts.push(json!({"id":id,"reply":reply}));
            *stored = json!(receipts);
        }
        crate::host::atomic_save(&self.path, &next)?;
        *data = next;
        Ok(reply)
    }

    fn snapshot(data: &Map<String, Value>) -> Value {
        let mut groups: Vec<_> = data["groups"].as_object().unwrap().values().collect();
        groups.sort_by_key(|group| std::cmp::Reverse(group["lastSeen"].as_u64().unwrap_or(0)));
        let resolved = groups
            .iter()
            .filter(|group| group["status"] == "resolved")
            .count();
        json!({"ok":true,"totalReports":data.get("totalReports").and_then(Value::as_u64).unwrap_or(0),
            "lastReportAt":data.get("lastReportAt").and_then(Value::as_u64).unwrap_or(0),
            "open":groups.len()-resolved,"resolved":resolved,"groups":groups})
    }

    fn change(&self, signature: Option<&str>, status: &str) -> Result<Value, StatusCode> {
        let mut data = self.data.lock().expect("diagnostics store");
        let mut next = data.clone();
        let groups = next
            .get_mut("groups")
            .and_then(Value::as_object_mut)
            .expect("validated diagnostics groups");
        let removed = if let Some(signature) = signature {
            let group = groups.get_mut(signature).ok_or(StatusCode::NOT_FOUND)?;
            group["status"] = json!(status);
            group["statusChangedAt"] = json!(now());
            None
        } else {
            let before = groups.len();
            groups.retain(|_, group| group["status"] != "resolved");
            Some(before - groups.len())
        };
        crate::host::atomic_save(&self.path, &next)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        *data = next;
        let mut result = Self::snapshot(&data);
        if let Some(removed) = removed {
            result["removed"] = json!(removed);
        }
        Ok(result)
    }
}

type Identity = dyn Fn(&HeaderMap) -> Option<Value> + Send + Sync;
type Changed = dyn Fn(&Value) + Send + Sync;
#[derive(Clone)]
struct Service {
    data: Arc<Diagnostics>,
    identity: Arc<Identity>,
    changed: Arc<Changed>,
}

pub fn router(
    data: Arc<Diagnostics>,
    identity: impl Fn(&HeaderMap) -> Option<Value> + Send + Sync + 'static,
    changed: impl Fn(&Value) + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/bug-report", post(report))
        .route("/admin/diagnostics", get(list))
        .route("/admin/diagnostics/export", get(export))
        .route("/admin/diagnostics/status", post(status))
        .route("/admin/diagnostics/clear-resolved", post(clear))
        .with_state(Service {
            data,
            identity: Arc::new(identity),
            changed: Arc::new(changed),
        })
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"ok":false,"error":message}))).into_response()
}
async fn body(request: Request) -> Result<Value, Response> {
    let bytes = to_bytes(request.into_body(), 128 * 1024)
        .await
        .map_err(|_| error(StatusCode::PAYLOAD_TOO_LARGE, "Laporan terlalu besar."))?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Data laporan tidak valid."))
}
async fn report(State(service): State<Service>, request: Request) -> Response {
    let Some(identity) = (service.identity)(request.headers()) else {
        return error(StatusCode::UNAUTHORIZED, "Sesi tidak tersedia.");
    };
    let input = match body(request).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    let data = service.data.clone();
    match tokio::task::spawn_blocking(move || data.record(&input, &identity)).await {
        Ok(Ok(result)) => {
            if result["replayed"] != true {
                (service.changed)(&result);
            }
            Json(json!({"ok":true,"signature":result["signature"],"count":result["count"]}))
                .into_response()
        }
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Gagal menyimpan laporan.",
        ),
    }
}
fn snapshot(service: &Service, headers: &HeaderMap, export: bool) -> Response {
    let identity = (service.identity)(headers).unwrap_or(Value::Null);
    if identity["isHost"] != true && identity["viewDiagnostics"] != true {
        return error(StatusCode::FORBIDDEN, "Akses diagnostik dibatasi HOST.");
    }
    let mut result = Diagnostics::snapshot(&service.data.data.lock().expect("diagnostics store"));
    if export {
        let ts = now();
        result.as_object_mut().unwrap().remove("ok");
        result["exportedAt"] = json!(ts);
        return (
            [(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=ChatX-Diagnostics-{ts}.json"),
            )],
            Json(result),
        )
            .into_response();
    }
    Json(result).into_response()
}
async fn list(State(service): State<Service>, headers: HeaderMap) -> Response {
    snapshot(&service, &headers, false)
}
async fn export(State(service): State<Service>, headers: HeaderMap) -> Response {
    snapshot(&service, &headers, true)
}
async fn status(State(service): State<Service>, request: Request) -> Response {
    if !(service.identity)(request.headers()).is_some_and(|user| user["isHost"] == true) {
        return error(StatusCode::FORBIDDEN, "Hanya HOST.");
    }
    let input = match body(request).await {
        Ok(input) => input,
        Err(response) => return response,
    };
    change(
        service,
        Some(field(&input, "signature", "", 120)),
        if input["status"] == "resolved" {
            "resolved"
        } else {
            "open"
        },
    )
    .await
}
async fn clear(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !(service.identity)(&headers).is_some_and(|user| user["isHost"] == true) {
        return error(StatusCode::FORBIDDEN, "Hanya HOST.");
    }
    change(service, None, "resolved").await
}
async fn change(service: Service, signature: Option<String>, status: &'static str) -> Response {
    match tokio::task::spawn_blocking(move || service.data.change(signature.as_deref(), status))
        .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(StatusCode::NOT_FOUND)) => error(StatusCode::NOT_FOUND, "Bug tidak ditemukan."),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Perubahan diagnostik gagal disimpan.",
        ),
    }
}
