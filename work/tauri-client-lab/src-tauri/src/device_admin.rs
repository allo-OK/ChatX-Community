use axum::{extract::{DefaultBodyLimit, State}, http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response}, routing::get, Json, Router};
use serde_json::{json, Value};

pub(super) fn router(access: super::Attachments) -> Router {
    Router::new().route("/admin/devices",get(list).post(decide))
        .layer(DefaultBodyLimit::max(64*1024)).with_state(access)
}

pub(super) fn authorized(access: &super::Attachments, headers: &HeaderMap, uri: &Uri) -> bool {
    let token=headers.get(header::AUTHORIZATION).and_then(|value|value.to_str().ok())
        .and_then(|value|value.strip_prefix("Bearer "))
        .or_else(||uri.query()?.split('&').find_map(|part|part.strip_prefix("kc_token=")));
    token.zip(access.admin.as_deref()).is_some_and(|(token,expected)|super::valid_admin_token(token) && expected.len()==token.len()
        && expected.bytes().zip(token.bytes()).fold(0,|diff,(a,b)|diff|(a^b))==0)
}

fn snapshot(registry: &super::DeviceRegistry) -> Value {
    let devices: Vec<_>=registry.records.iter().map(|(id,record)| {
        let mut record=record.as_object().cloned().unwrap_or_default();
        for field in ["keyHash","deviceKey","sessionToken","token"] {record.remove(field);}
        record.insert("deviceId".into(),json!(id));
        Value::Object(record)
    }).collect();
    json!({"ok":true,"devices":devices,"pending":[]})
}

fn denied() -> Response {
    (StatusCode::FORBIDDEN,Json(json!({"ok":false,"error":"Pengaturan perangkat hanya untuk HOST"}))).into_response()
}

async fn list(State(access): State<super::Attachments>, headers: HeaderMap, uri: Uri) -> Response {
    if !authorized(&access,&headers,&uri) {return denied();}
    Json(snapshot(&access.devices.lock().expect("device registry"))).into_response()
}

async fn decide(State(access): State<super::Attachments>, headers: HeaderMap, uri: Uri, Json(input): Json<Value>) -> Response {
    if !authorized(&access,&headers,&uri) {return denied();}
    let registry=access.devices.clone();
    let result=tokio::task::spawn_blocking(move || -> Result<_,String> {
        if input["requestId"].as_str().is_some_and(|id|!id.is_empty()) {return Err("Permintaan tidak ditemukan".into());}
        let id=input["deviceId"].as_str().filter(|id|!id.is_empty()).ok_or("Pilih perangkat")?.to_owned();
        let mut registry=registry.lock().expect("device registry");
        registry.set_device_revoked(&id,input["remove"]!=true)?;
        Ok((id,snapshot(&registry)))
    }).await;
    match result {
        Ok(Ok((id,reply)))=>{
            for peer in access.io.sockets() {
                if peer.extensions.get::<super::Presence>().is_some_and(|presence|presence.public["deviceId"]==id) {
                    let _=peer.disconnect();
                }
            }
            Json(reply).into_response()
        }
        Ok(Err(error))=>(StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":error}))).into_response(),
        Err(_)=>(StatusCode::INTERNAL_SERVER_ERROR,Json(json!({"ok":false,"error":"Pengaturan perangkat terhenti"}))).into_response(),
    }
}
