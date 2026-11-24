use axum::{extract::{Request, State}, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response},
    routing::{get, post}, Json, Router};
use serde_json::{json, Map, Value};
use std::{io, path::PathBuf, sync::{Arc, Mutex}};

const KEYS: [&str; 4] = ["manageUsers", "manageVideos", "manageJobs", "viewDiagnostics"];
const PERMISSIONS: [&str; 11] = ["personal","editor","everyone","sendChat","sendFiles","downloadFiles","designAI","tagPrintTest","tagDesignRevision","tagDesignRequest","banned"];
type Identity = dyn Fn(&HeaderMap) -> Option<(String, bool, String)> + Send + Sync;
type Users = dyn Fn() -> Vec<Value> + Send + Sync;
type Changed = dyn Fn(&str, Value) + Send + Sync;
#[path = "moderation.rs"]
mod moderation;

pub(crate) fn storage_audit(id:&str,kind:&str,detail:Value)->io::Result<()> {
    moderation::storage_audit(id,kind,detail)
}
pub(crate) fn storage_audit_ready()->io::Result<()> {moderation::storage_audit_ready()}

pub(crate) fn persist_device_registry(path: &std::path::Path, current: &mut Map<String,Value>, next: Map<String,Value>, detail: Value) -> Result<(),String> {
    moderation::persist_file(path,current,next,"device-access",detail)
}

pub struct Access {
    path: PathBuf,
    data: Mutex<Map<String, Value>>,
}

impl Access {
    pub(crate) fn reserves_profile(&self, name: &str) -> bool {
        let key = crate::profiles::key(name);
        let data = self.data.lock().expect("known members");
        data.get("profilePolicies").and_then(|policies| policies.get(&key)).is_some()
            || data.get("knownUsers").and_then(Value::as_object).is_some_and(|users| {
                users.values().any(|user| user["name"].as_str()
                    .is_some_and(|name| crate::profiles::key(name) == key))
            })
    }

    pub(crate) fn directory(&self, current: Vec<Value>) -> Vec<Value> {
        let data=self.data.lock().expect("known members");
        let mut users=std::collections::BTreeMap::<String,Value>::new();
        if let Some(known)=data.get("knownUsers").and_then(Value::as_object) {
            for (id,record) in known {
                if id.is_empty() || record["name"].as_str().is_none_or(|name|name.trim().is_empty()) {continue}
                users.insert(id.clone(),json!({"deviceId":id,"name":record["name"],"pc":record["pc"],
                    "isHost":record["isHost"]==true,"online":false,"lastSeen":record["lastSeen"],
                    "teamRole":record["teamRole"].as_str().unwrap_or("EDITOR"),"version":record["version"]}));
            }
        }
        for mut user in current {
            let Some(id)=user["deviceId"].as_str().filter(|id|!id.is_empty()).map(str::to_owned) else {continue};
            if let Some(old)=users.get(&id) {
                user["isHost"]=json!(user["isHost"]==true || old["isHost"]==true);
                for field in ["pc","version","lastSeen"] {
                    if user[field].is_null() {user[field]=old[field].clone();}
                }
            }
            users.insert(id,user);
        }
        users.into_values().collect()
    }

    pub fn activity_override(&self, subject: &str, viewer: &str, name: &str, host: bool) -> bool {
        let admin=host || self.policy(name,viewer,host)["moderationRole"]=="ADMIN";
        let data=self.data.lock().expect("activity visibility");
        let policy=data.get("activityVisibility").and_then(|rules| rules.get(subject)).unwrap_or(&Value::Null);
        (admin && policy["includeAdmins"]==true) || policy["viewerDeviceIds"].as_array()
            .is_some_and(|ids|ids.iter().any(|id|id.as_str()==Some(viewer)))
    }

    pub fn open(path: PathBuf) -> io::Result<Self> {
        let data = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, data: Mutex::new(data) })
    }

    pub fn scopes(&self, name: &str, host: bool) -> Value {
        let data = self.data.lock().expect("cohost policies");
        let key = crate::profiles::key(name);
        let mut scopes = Map::new();
        for scope in KEYS {
            scopes.insert(scope.into(), json!(host || data.get("profilePolicies").is_some_and(|policies|
                policies[&key]["cohost"][scope] == true && policies[&key]["permissions"]["banned"] != true)));
        }
        Value::Object(scopes)
    }

    pub fn jobs(&self) -> Vec<String> {
        self.data.lock().expect("cohost policies").get("jobNames").and_then(Value::as_array)
            .map(|jobs| jobs.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_else(|| ["EDITOR","ADMIN","QC","FLEX"].map(str::to_owned).to_vec())
    }

    pub fn assigned_job(&self, name: &str, device: &str) -> Option<String> {
        let data = self.data.lock().expect("cohost policies");
        data.get("profilePolicies").and_then(|policies| policies[crate::profiles::key(name)]["teamRole"].as_str())
            .or_else(|| data.get("users").and_then(|users| users[device]["teamRole"].as_str()))
            .or_else(|| data.get("knownUsers").and_then(|users| {
                let known = &users[device];
                known["name"].as_str().filter(|saved| crate::profiles::key(saved) == crate::profiles::key(name))?;
                known["teamRole"].as_str()
            }))
            .map(str::trim).filter(|role| !role.is_empty() && role.chars().count() <= 40)
            .map(str::to_uppercase)
    }

    pub fn permissions(&self, name: &str, host: bool) -> Value {
        self.policy(name, "", host)["permissions"].clone()
    }

    pub fn policy(&self, name: &str, device: &str, host: bool) -> Value {
        let data = self.data.lock().expect("cohost policies");
        let key = crate::profiles::key(name);
        let device_raw = &data.get("users").unwrap_or(&Value::Null)[device];
        let profile_raw = &data.get("profilePolicies").unwrap_or(&Value::Null)[&key];
        let mut raw = device_raw.as_object().cloned().unwrap_or_default();
        raw.extend(profile_raw.as_object().cloned().unwrap_or_default());
        let mut per = device_raw["permissions"].as_object().cloned().unwrap_or_default();
        per.extend(profile_raw["permissions"].as_object().cloned().unwrap_or_default());
        let raw = Value::Object(raw);
        let role = match raw["moderationRole"].as_str().unwrap_or("EDITOR").to_uppercase().as_str() {
            "ADMIN" => "ADMIN", "CUSTOM" => "CUSTOM", _ => "EDITOR",
        };
        let role_id = raw["roleTemplateId"].as_str().unwrap_or(role);
        let template = &data.get("roles").unwrap_or(&Value::Null)[role_id];
        let admin = role == "ADMIN";
        let permissions = Value::Object(PERMISSIONS.into_iter().map(|permission| {
            let default = match permission { "banned" => false, "editor" | "everyone" => admin && !template["permissions"].is_object(), _ => true };
            let configured = per.get(permission).and_then(Value::as_bool)
                .or_else(|| template["permissions"][permission].as_bool()).unwrap_or(default);
            let value = if host { permission != "banned" } else { configured };
            (permission.into(), json!(value))
        }).collect());
        let level = raw["mentionLevel"].as_str().filter(|level| !level.is_empty())
            .or_else(|| (!host).then(|| template["mentionLevel"].as_str()).flatten())
            .unwrap_or(if host { "fullscreen" } else if admin { "medium" } else { "notice" });
        let level = if ["standard","notice","medium","fullscreen"].contains(&level) { level } else { "notice" };
        let mode = raw["fullscreenMode"].as_str().filter(|mode| !mode.is_empty())
            .or_else(|| (!host).then(|| template["fullscreenMode"].as_str()).flatten())
            .unwrap_or(if host { "open-chat" } else { "video" });
        let mode = if mode == "open-chat" { "open-chat" } else { "video" };
        let custom = if role == "CUSTOM" { template["name"].as_str().or_else(|| raw["customRoleName"].as_str()).unwrap_or("Custom") } else { "" };
        json!({"permissions":permissions,"tagVideoEnabled":raw["tagVideoEnabled"].as_bool().unwrap_or(level == "fullscreen" && mode == "video"),
            "receiveTagVideoEnabled":raw["receiveTagVideoEnabled"].as_bool().unwrap_or(true),
            "receiveTagSoundEnabled":raw["receiveTagSoundEnabled"].as_bool().unwrap_or(true),
            "mentionLevel":level,"fullscreenMode":mode,"mutedUntil":if host {0.0} else {raw["mutedUntil"].as_f64().unwrap_or_default()},
            "bypassAppMute":host || raw["bypassAppMute"].as_bool().or_else(|| template["bypassAppMute"].as_bool()).unwrap_or(true),
            "bypassWindowsMute":host || raw["bypassWindowsMute"].as_bool().or_else(|| template["bypassWindowsMute"].as_bool()).unwrap_or(admin),
            "moderationRole":if host {"DEVELOPER"} else {role},"roleTemplateId":role_id,"customRoleName":custom,
            "roleLabel":if host {"Developer"} else if role == "CUSTOM" {custom} else if admin {"Admin"} else {"Editor"}})
    }

    pub fn tag_spam_policy(&self) -> Value {
        self.data.lock().expect("cohost policies").get("tagSpamPolicy").cloned().unwrap_or(Value::Null)
    }

    pub fn consume_everyone(&self, device: &str, now: u64) -> Result<Option<u64>, String> {
        let mut data = self.data.lock().expect("cohost policies");
        let last = data.get("lastEveryone").map(|values| values[device].as_u64().unwrap_or_default()).unwrap_or_default();
        if now.saturating_sub(last) < 300000 { return Ok(Some(300000 - now.saturating_sub(last))) }
        let mut next = data.clone();
        let values = next.entry("lastEveryone").or_insert_with(|| json!({})).as_object_mut().ok_or("Pengaturan jeda tag rusak.")?;
        values.insert(device.into(), json!(now));
        crate::host::atomic_save(&self.path, &next).map_err(|error| format!("Jeda tag gagal disimpan: {error}"))?;
        *data = next;
        Ok(None)
    }

    fn change_member(&self, name: &str, input: &Value) -> Result<(), String> {
        if input.as_object().is_none_or(|fields| fields.keys().any(|key| !["name","permissions","tagVideoEnabled","receiveTagVideoEnabled","receiveTagSoundEnabled"].contains(&key.as_str()))) {
            return Err("Perubahan di luar izin Co-HOST.".into());
        }
        let updates = input.get("permissions").map(|value| value.as_object().ok_or("Pengaturan izin tidak valid.")).transpose()?;
        if updates.is_some_and(|updates| updates.iter().any(|(key,value)| !PERMISSIONS.contains(&key.as_str()) || !value.is_boolean())) {
            return Err("Perubahan di luar izin Co-HOST.".into());
        }
        if ["tagVideoEnabled","receiveTagVideoEnabled","receiveTagSoundEnabled"].iter().any(|key| input.get(key).is_some_and(|value| !value.is_boolean())) {
            return Err("Pilihan video tag tidak valid.".into());
        }
        let mut data = self.data.lock().expect("cohost policies");
        let mut next = data.clone();
        let key = crate::profiles::key(name);
        let policies = next.entry("profilePolicies").or_insert_with(|| json!({})).as_object_mut().ok_or("Pengaturan anggota rusak.")?;
        let policy = policies.entry(key).or_insert_with(|| json!({})).as_object_mut().ok_or("Pengaturan anggota rusak.")?;
        if let Some(updates) = updates {
            let permissions = policy.entry("permissions").or_insert_with(|| json!({})).as_object_mut().ok_or("Pengaturan izin rusak.")?;
            permissions.extend(updates.clone());
        }
        if let Some(video) = input.get("tagVideoEnabled") { policy.insert("tagVideoEnabled".into(), video.clone()); }
        for key in ["receiveTagVideoEnabled","receiveTagSoundEnabled"] {
            if let Some(value) = input.get(key) { policy.insert(key.into(),value.clone()); }
        }
        crate::host::atomic_save(&self.path, &next).map_err(|error| format!("Izin anggota gagal disimpan: {error}"))?;
        *data = next;
        Ok(())
    }

    fn save_jobs(&self, input: &Value) -> Result<Vec<String>, String> {
        let extra = input["names"].as_array().ok_or("Daftar Job Desk tidak valid.")?;
        if extra.len() > 40 { return Err("Maksimal 40 pekerjaan tambahan.".into()); }
        let mut jobs = ["EDITOR","ADMIN","QC","FLEX"].map(str::to_owned).to_vec();
        for name in extra {
            let name = name.as_str().ok_or("Nama pekerjaan harus berupa teks.")?;
            let cleaned: String = name.chars().map(|ch| if matches!(ch, '\r' | '\n' | '\t' | '<' | '>') { ' ' } else { ch }).collect();
            let name: String = cleaned.trim().chars().take(40).collect::<String>().to_uppercase();
            if !name.is_empty() && !jobs.contains(&name) { jobs.push(name); }
        }
        let mut data = self.data.lock().expect("cohost policies");
        let mut next = data.clone();
        next.insert("jobNames".into(), json!(jobs));
        crate::host::atomic_save(&self.path, &next).map_err(|error| format!("Job Desk gagal disimpan: {error}"))?;
        *data = next;
        Ok(jobs)
    }

    fn grant(&self, name: &str, scopes: &Value) -> Result<(), String> {
        let mut data = self.data.lock().expect("cohost policies");
        let mut next = data.clone();
        let key = crate::profiles::key(name);
        let policies = next.entry("profilePolicies").or_insert_with(|| json!({})).as_object_mut()
            .ok_or("Pengaturan izin anggota tidak valid.")?;
        let policy = policies.entry(key).or_insert_with(|| json!({})).as_object_mut()
            .ok_or("Pengaturan anggota tidak valid.")?;
        let cleaned: Map<_, _> = KEYS.into_iter().map(|scope| (scope.into(), json!(scopes[scope] == true))).collect();
        policy.insert("cohost".into(), Value::Object(cleaned));
        crate::host::atomic_save(&self.path, &next).map_err(|error| format!("Izin co-host gagal disimpan: {error}"))?;
        *data = next;
        Ok(())
    }
}

#[derive(Clone)]
struct Service {
    access: Arc<Access>, identity: Arc<Identity>, users: Arc<Users>, changed: Arc<Changed>,
}

impl Service {
    fn users(&self) -> Vec<Value> {self.access.directory((self.users)())}
}

pub fn router(access: Arc<Access>, identity: impl Fn(&HeaderMap) -> Option<(String, bool, String)> + Send + Sync + 'static,
    users: impl Fn() -> Vec<Value> + Send + Sync + 'static, changed: impl Fn(&str, Value) + Send + Sync + 'static) -> Router {
    Router::new().route("/cohost", get(snapshot)).route("/cohost/users", get(users_list))
        .route("/cohost/grant", post(grant)).route("/cohost/jobs", post(save_jobs)).route("/cohost/user", post(change_member))
        .merge(moderation::routes())
        .with_state(Service { access, identity: Arc::new(identity), users: Arc::new(users), changed: Arc::new(changed) })
}

async fn change_member(State(service): State<Service>, request: Request) -> Response {
    let Some((actor, host, device)) = (service.identity)(request.headers()) else { return denied() };
    if service.access.policy(&actor,&device,host)["permissions"]["banned"] == true || service.access.scopes(&actor, host)["manageUsers"] != true { return denied() }
    let result: Result<Value, String> = async {
        let bytes = axum::body::to_bytes(request.into_body(), 8192).await.map_err(|_| "Data izin terlalu besar.")?;
        let input: Value = serde_json::from_slice(&bytes).map_err(|_| "Data izin tidak valid.")?;
        let name = input["name"].as_str().filter(|name| !name.trim().is_empty()).ok_or("Pilih anggota CLIENT yang valid.")?;
        let users = service.users();
        let members: Vec<_> = users.iter().filter(|user| user["name"].as_str().is_some_and(|member| member.trim().eq_ignore_ascii_case(name.trim()))).collect();
        if members.is_empty() || members.iter().any(|user| user["isHost"] == true)
            || (!host && (actor.trim().eq_ignore_ascii_case(name.trim()) || service.access.scopes(name, false).as_object()
                .is_some_and(|scopes| scopes.values().any(|scope| scope == true)))) {
            return Err("Co-HOST hanya dapat mengelola anggota di bawahnya.".into());
        }
        service.access.change_member(name, &input)?;
        (service.changed)(name, service.access.scopes(name, false));
        Ok(json!({"ok":true}))
    }.await;
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

async fn save_jobs(State(service): State<Service>, request: Request) -> Response {
    let Some((name, host, device)) = (service.identity)(request.headers()) else { return denied() };
    if service.access.policy(&name,&device,host)["permissions"]["banned"] == true || service.access.scopes(&name, host)["manageJobs"] != true { return denied() }
    let result: Result<Value, String> = async {
        let bytes = axum::body::to_bytes(request.into_body(), 8192).await.map_err(|_| "Daftar Job Desk terlalu besar.")?;
        let input: Value = serde_json::from_slice(&bytes).map_err(|_| "Daftar Job Desk tidak valid.")?;
        let jobs = service.access.save_jobs(&input)?;
        (service.changed)("", json!(jobs));
        Ok(json!({"ok":true,"names":jobs}))
    }.await;
    match result {
        Ok(data) => Json(data).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

fn denied() -> Response {
    (StatusCode::FORBIDDEN, Json(json!({"ok":false,"error":"HOST belum memberi izin pengelolaan ini."}))).into_response()
}

async fn snapshot(State(service): State<Service>, headers: HeaderMap) -> Response {
    let Some((name, host, device)) = (service.identity)(&headers) else { return denied() };
    let data = service.access.data.lock().expect("cohost policies");
    let jobs = data.get("jobNames").cloned().unwrap_or_else(|| json!(["EDITOR","ADMIN","QC","FLEX"]));
    drop(data);
    Json(json!({"ok":true,"scopes":service.access.scopes(&name, host),
        "permissions":service.access.policy(&name,&device,host)["permissions"],"jobNames":jobs})).into_response()
}

async fn users_list(State(service): State<Service>, headers: HeaderMap) -> Response {
    let Some((name, host, device)) = (service.identity)(&headers) else { return denied() };
    if service.access.policy(&name,&device,host)["permissions"]["banned"] == true || service.access.scopes(&name, host)["manageUsers"] != true { return denied() }
    let users: Vec<_> = service.users().into_iter().map(|mut user| {
        let policy = service.access.policy(user["name"].as_str().unwrap_or_default(), user["deviceId"].as_str().unwrap_or_default(), user["isHost"] == true);
        user["cohost"] = service.access.scopes(user["name"].as_str().unwrap_or_default(), user["isHost"] == true);
        user["permissions"] = policy["permissions"].clone();
        user["tagVideoEnabled"] = policy["tagVideoEnabled"].clone();
        user["receiveTagVideoEnabled"] = policy["receiveTagVideoEnabled"].clone();
        user["receiveTagSoundEnabled"] = policy["receiveTagSoundEnabled"].clone();
        user
    }).collect();
    Json(json!({"ok":true,"users":users})).into_response()
}

async fn grant(State(service): State<Service>, request: Request) -> Response {
    let Some((_, true, _)) = (service.identity)(request.headers()) else { return denied() };
    let result: Result<Value, String> = async {
        let bytes = axum::body::to_bytes(request.into_body(), 8192).await.map_err(|_| "Data izin terlalu besar.")?;
        let input: Value = serde_json::from_slice(&bytes).map_err(|_| "Data izin tidak valid.")?;
        let name = input["name"].as_str().filter(|name| !name.trim().is_empty()).ok_or("Pilih anggota CLIENT yang valid.")?;
        let users = service.users();
        let members: Vec<_> = users.iter().filter(|user| user["name"].as_str().is_some_and(|member| member.trim().eq_ignore_ascii_case(name.trim()))).collect();
        if members.is_empty() || members.iter().any(|user| user["isHost"] == true) { return Err("Pilih anggota CLIENT yang valid.".into()); }
        service.access.grant(name, &input["scopes"])?;
        (service.changed)(name, service.access.scopes(name, false));
        Ok(json!({"ok":true}))
    }.await;
    match result {
        Ok(data) => Json(data).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"ok":false,"error":error}))).into_response(),
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn preserves_original_role_and_notification_policies() {
        let fixtures: Value = serde_json::from_str(include_str!("../../message-tags-fixtures.json")).unwrap();
        let directory = std::env::temp_dir().join(format!("kc-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let path = directory.join("moderation.json");
            crate::host::atomic_save(&path, case["moderation"].as_object().unwrap()).unwrap();
            let access = Access::open(path).unwrap();
            let actual = access.policy(case["name"].as_str().unwrap(),case["device"].as_str().unwrap(),case["host"] == true);
            for (key, expected) in case["expected"].as_object().unwrap() {
                if key == "mutedUntil" { assert_eq!(actual[key].as_f64(),expected.as_f64()); }
                else { assert_eq!(&actual[key],expected,"{}: {key}",case["name"]); }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
