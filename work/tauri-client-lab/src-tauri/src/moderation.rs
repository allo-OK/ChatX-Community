use super::*;
use std::io::Write;
static AUDIT_LOCK: Mutex<()> = Mutex::new(());
pub(super) fn storage_audit_ready()->io::Result<()> {
    let _guard=AUDIT_LOCK.lock().expect("audit log");
    match std::fs::File::open("audit-log.json") {
        Ok(file)=>{let _:Vec<Value>=serde_json::from_reader(file).map_err(io::Error::other)?;Ok(())},
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Ok(()),Err(error)=>Err(error),
    }
}

pub(super) fn storage_audit(id:&str,kind:&str,detail:Value)->io::Result<()> {
    let _guard=AUDIT_LOCK.lock().expect("audit log");
    let path=std::path::Path::new("audit-log.json");
    let mut audit:Vec<Value>=match std::fs::File::open(path) {
        Ok(file)=>serde_json::from_reader(file).map_err(io::Error::other)?,
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Vec::new(),
        Err(error)=>return Err(error),
    };
    if audit.iter().any(|entry|entry["id"]==id) {return Ok(())}
    audit.push(json!({"id":id,"ts":now(),"type":kind,"actor":"HOST","detail":detail}));
    if audit.len()>1000 {audit.drain(..audit.len()-1000);}
    crate::host::atomic_write(path,|file|file.write_all(&serde_json::to_vec_pretty(&audit)?))
}

pub fn routes() -> Router<Service> {
    Router::new().route("/admin/moderation", get(snapshot))
        .route("/admin/moderation/user", post(update_user))
        .route("/admin/moderation/role", post(update_role))
        .route("/admin/moderation/tag-spam", post(update_spam))
        .route("/admin/activity-visibility", get(activity_visibility).post(update_activity_visibility))
}

async fn activity_visibility(State(service): State<Service>, request: Request) -> Response {
    if !host(&service,request.headers()) { return denied() }
    let uri=tauri::Url::parse(&format!("http://localhost/?{}",request.uri().query().unwrap_or_default())).expect("valid local query URL");
    let query=uri.query_pairs().collect::<std::collections::HashMap<_,_>>();
    let id=query.get("subjectDeviceId").map(|value|value.as_ref()).unwrap_or_default();
    let users=service.users();
    let Some(subject)=users.iter().find(|user|user["deviceId"]==id) else {
        return (StatusCode::NOT_FOUND,Json(json!({"ok":false,"error":"User tidak ditemukan"}))).into_response()
    };
    let mut viewers=users.iter().filter(|user|user["deviceId"]!=id && user["isHost"]!=true).cloned().map(|mut user| {
        user["moderationRole"]=service.access.policy(user["name"].as_str().unwrap_or_default(),user["deviceId"].as_str().unwrap_or_default(),false)["moderationRole"].clone();
        user
    }).collect::<Vec<_>>();
    viewers.sort_by_key(|user|user["name"].as_str().unwrap_or_default().to_lowercase());
    let data=service.access.data.lock().expect("activity visibility");
    let policy=data.get("activityVisibility").and_then(|rules|rules.get(id)).cloned().unwrap_or_else(||json!({"includeAdmins":false,"viewerDeviceIds":[]}));
    Json(json!({"ok":true,"subject":subject,"users":viewers,"policy":policy})).into_response()
}

async fn update_activity_visibility(State(service): State<Service>, request: Request) -> Response {
    if !host(&service,request.headers()) {return denied()}
    let input=match input(request).await {Ok(input)=>input,Err(error)=>return response(Err(error))};
    let users=service.users();
    let id=input["subjectDeviceId"].as_str().unwrap_or_default();
    let Some(subject)=users.iter().find(|user|user["deviceId"]==id && user["isHost"]!=true) else {
        return (StatusCode::NOT_FOUND,Json(json!({"ok":false,"error":"Editor CLIENT tidak ditemukan"}))).into_response()
    };
    let result=async {
        let mut ids=Vec::<String>::new();
        for value in input["viewerDeviceIds"].as_array().into_iter().flatten() {
            if let Some(viewer)=value.as_str().filter(|viewer|*viewer!=id && users.iter().any(|user|user["deviceId"]==*viewer && user["isHost"]!=true)) {
                if !ids.iter().any(|id|id==viewer) && ids.len()<100 {ids.push(viewer.to_owned());}
            }
        }
        let include=input["includeAdmins"]==true;
        let policy=json!({"includeAdmins":include,"viewerDeviceIds":ids,"updatedAt":now()});
        {
            let mut data=service.access.data.lock().expect("activity visibility");
            let mut next=data.clone();
            let rules=next.entry("activityVisibility").or_insert_with(||json!({})).as_object_mut().ok_or("Pengaturan aktivitas rusak")?;
            if !include && ids.is_empty() {rules.remove(id);} else {rules.insert(id.into(),policy.clone());}
            persist(&service.access,&mut data,next,"activity-visibility",json!({"deviceId":id,"name":subject["name"],"includeAdmins":include,"viewerDeviceIds":ids}))?;
        }
        announce(&service);
        Ok(json!({"ok":true,"policy":if !include && ids.is_empty() {json!({"includeAdmins":false,"viewerDeviceIds":[]})} else {policy}}))
    }.await;
    response(result)
}

fn defaults(role: &str) -> Value {
    let admin = role == "ADMIN";
    json!({"permissions":{"personal":true,"editor":admin,"everyone":admin},
        "mentionLevel":if admin {"medium"} else {"notice"},"fullscreenMode":"video", "bypassAppMute":true,"bypassWindowsMute":admin})
}
fn permissions(previous: &Value, input: &Value) -> Value {
    Value::Object(PERMISSIONS.into_iter().map(|key| (key.into(), json!(input[key].as_bool()
        .or_else(|| previous[key].as_bool()).unwrap_or(!["editor","everyone","banned"].contains(&key))))).collect())
}
fn level(value: &Value) -> &str { value.as_str().filter(|v| ["standard","notice","medium","fullscreen"].contains(v)).unwrap_or("notice") }
fn mode(value: &Value) -> &str { if value == "open-chat" {"open-chat"} else {"video"} }
fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn spam(input: &Value) -> Value {
    let clamp = |key: &str, min: f64, max: f64, default: f64| {
        input.get(key).and_then(|value| match value {
            Value::Null => Some(0.0), Value::Bool(v) => Some(if *v {1.0} else {0.0}),
            Value::String(v) => if v.trim().is_empty() {Some(0.0)} else {v.parse().ok()}, _ => value.as_f64(),
        }).filter(|v| v.is_finite()).unwrap_or(default).round().max(min).min(max) as u64
    };
    json!({"maxPer10Seconds":clamp("maxPer10Seconds",1.0,10.0,2.0),"maxPerMinute":clamp("maxPerMinute",2.0,30.0,6.0),"blockSeconds":clamp("blockSeconds",5.0,300.0,30.0)})
}

fn persist(access: &Access, data: &mut Map<String,Value>, next: Map<String,Value>, kind: &str, detail: Value) -> Result<(),String> {
    persist_file(&access.path,data,next,kind,detail)
}

pub(super) fn persist_file(path: &std::path::Path, data: &mut Map<String,Value>, next: Map<String,Value>, kind: &str, detail: Value) -> Result<(),String> {
    let _guard = AUDIT_LOCK.lock().expect("audit log");
    let audit_path = path.with_file_name("audit-log.json");
    let mut audit: Vec<Value> = match std::fs::File::open(&audit_path) {
        Ok(file) => serde_json::from_reader(file).map_err(|_| "Log audit rusak; pengaturan belum diubah.".to_owned())?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("Log audit gagal dibaca: {error}")),
    };
    audit.push(json!({"id":uuid::Uuid::new_v4().to_string(),"ts":now(),"type":kind,"actor":"HOST","detail":detail}));
    if audit.len() > 1000 { audit.drain(..audit.len()-1000); }
    crate::host::atomic_save(path, &next).map_err(|error| format!("Pengaturan belum tersimpan: {error}"))?;
    if let Err(error) = crate::host::atomic_write(&audit_path, |file| file.write_all(&serde_json::to_vec_pretty(&audit)?)) {
        if let Err(rollback) = crate::host::atomic_save(path, data) {
            *data = next;
            return Err(format!("Pengaturan tersimpan tetapi audit gagal ({error}); pemulihan pengaturan gagal ({rollback})."));
        }
        return Err(format!("Audit gagal disimpan; perubahan dibatalkan: {error}"));
    }
    *data = next;
    Ok(())
}

fn snapshot_value(service: &Service) -> Result<Value,String> {
    let source = service.users();
    let mut users = std::collections::BTreeMap::<String,Value>::new();
    for mut user in source {
        let name = user["name"].as_str().unwrap_or_default();
        if name.is_empty() { continue }
        let key = crate::profiles::key(name);
        user["profileKey"] = json!(key);
        match users.get_mut(&key) {
            Some(previous) => {
                let host = previous["isHost"] == true || user["isHost"] == true;
                if (user["online"] == true && previous["online"] != true)
                    || (user["online"] == previous["online"] && user["lastSeen"].as_u64().unwrap_or_default() > previous["lastSeen"].as_u64().unwrap_or_default()) { *previous = user; }
                previous["isHost"] = json!(host);
            }
            None => {users.insert(key,user);}
        }
    }
    let mut users: Vec<_> = users.into_values().map(|mut user| {
        let name = user["name"].as_str().unwrap_or_default();
        let mut policy = service.access.policy(name,user["deviceId"].as_str().unwrap_or_default(),user["isHost"] == true);
        policy["cohost"] = service.access.scopes(name,user["isHost"] == true);
        user["policy"] = policy;
        user
    }).collect();
    users.sort_by(|a,b| (b["online"] == true).cmp(&(a["online"] == true)).then_with(|| b["lastSeen"].as_u64().cmp(&a["lastSeen"].as_u64())));
    let data = service.access.data.lock().expect("moderation policy");
    let saved_roles = data.get("roles").and_then(Value::as_object);
    let mut roles = Vec::new();
    for id in ["ADMIN","EDITOR"] {
        let mut role = defaults(id);
        role["id"] = json!(id);
        role["name"] = json!(if id == "ADMIN" {"Admin"} else {"Editor"});
        if let Some(saved) = saved_roles.and_then(|roles| roles.get(id)).and_then(Value::as_object) { role.as_object_mut().unwrap().extend(saved.clone()); }
        roles.push(role);
    }
    if let Some(saved) = saved_roles {
        for (id,role) in saved.iter().filter(|(id,_)| !["ADMIN","EDITOR"].contains(&id.as_str())) {
            let mut role = role.clone();
            role["id"] = json!(id);
            roles.push(role);
        }
    }
    let tag_spam = spam(data.get("tagSpamPolicy").unwrap_or(&Value::Null));
    drop(data);
    let audit: Vec<Value> = match std::fs::File::open(service.access.path.with_file_name("audit-log.json")) {
        Ok(file) => serde_json::from_reader(file).map_err(|_| "Log audit rusak.".to_owned())?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("Log audit gagal dibaca: {error}")),
    };
    Ok(json!({"ok":true,"users":users,"roles":roles,"audit":audit.into_iter().rev().take(100).collect::<Vec<_>>(),
        "cooldownMinutes":5,"maxPersonalMentions":5,"tagSpamPolicy":tag_spam}))
}

fn response(result: Result<Value,String>) -> Response {
    match result { Ok(value) => Json(value).into_response(), Err(error) => (StatusCode::BAD_REQUEST,Json(json!({"ok":false,"error":error}))).into_response() }
}
fn host(service: &Service, headers: &HeaderMap) -> bool { (service.identity)(headers).is_some_and(|(_,host,_)| host) }
async fn input(request: Request) -> Result<Value,String> {
    let bytes = axum::body::to_bytes(request.into_body(),32768).await.map_err(|_| "Pengaturan terlalu besar atau terputus.".to_owned())?;
    serde_json::from_slice(&bytes).map_err(|_| "Format pengaturan tidak valid.".to_owned())
}
async fn snapshot(State(service): State<Service>, headers: HeaderMap) -> Response {
    if !host(&service,&headers) { return denied() }
    let result = tokio::task::spawn_blocking(move || snapshot_value(&service)).await;
    response(result.unwrap_or_else(|_| Err("Moderation gagal dimuat.".into())))
}
fn announce(service: &Service) { (service.changed)("",json!({"refreshModeration":true})); }

async fn update_spam(State(service): State<Service>, request: Request) -> Response {
    if !host(&service,request.headers()) { return denied() }
    let result = async {
        let policy = spam(&input(request).await?);
        {
            let mut data = service.access.data.lock().expect("moderation policy");
            let mut next = data.clone();
            next.insert("tagSpamPolicy".into(),policy.clone());
            persist(&service.access,&mut data,next,"tag-spam-policy",json!({"policy":policy}))?;
        }
        announce(&service);
        snapshot_value(&service)
    }.await;
    response(result)
}

async fn update_user(State(service): State<Service>, request: Request) -> Response {
    if !host(&service,request.headers()) { return denied() }
    let result = async {
        let input = input(request).await?;
        let users = service.users();
        let name = input["profileName"].as_str().filter(|name| !name.is_empty()).or_else(|| users.iter()
            .find(|user| user["deviceId"] == input["deviceId"] && !input["deviceId"].is_null()).and_then(|user| user["name"].as_str())).ok_or("Profil user tidak valid")?;
        let key = crate::profiles::key(name);
        let members: Vec<_> = users.iter().filter(|user| crate::profiles::key(user["name"].as_str().unwrap_or_default()) == key).collect();
        if members.is_empty() {return Err("Profil user tidak valid".into())}
        let is_host = members.iter().any(|user| user["isHost"] == true);
        {
            let mut data = service.access.data.lock().expect("moderation policy");
            let mut next = data.clone();
            let mut current = data.get("profilePolicies").and_then(|policies| policies.get(&key)).and_then(Value::as_object).cloned().unwrap_or_default();
            for field in ["tagVideoEnabled","mentionLevel","fullscreenMode"] {
                if let Some(value) = input.get(field) {
                    let value = match field { "tagVideoEnabled" => value.as_bool().map(Value::Bool), "mentionLevel" => Some(json!(level(value))), _ => Some(json!(mode(value))) };
                    if let Some(value) = value {current.insert(field.into(),value);}
                }
            }
            if !is_host {
                if let Some(raw) = input.get("moderationRole") {
                    let role = match raw.as_str().unwrap_or_default().to_uppercase().as_str() {"ADMIN"=>"ADMIN","CUSTOM"=>"CUSTOM",_=>"EDITOR"};
                    let old = current.get("moderationRole").and_then(Value::as_str).unwrap_or("EDITOR").to_owned();
                    current.insert("moderationRole".into(),json!(role));
                    current.remove("roleTemplateId");
                    if role == "CUSTOM" { current.insert("customRoleName".into(),json!(input["customRoleName"].as_str().unwrap_or("Custom").trim().chars().take(40).collect::<String>())); }
                    else {current.remove("customRoleName");}
                    if role != old || input["applyRoleDefaults"] == true {current.extend(defaults(role).as_object().unwrap().clone());}
                }
                if let Some(value) = input.get("customRoleName").and_then(Value::as_str) {
                    if current.get("moderationRole").is_some_and(|role| role == "CUSTOM") {current.insert("customRoleName".into(),json!(if value.trim().is_empty() {"Custom".into()} else {value.trim().chars().take(40).collect::<String>()}));}
                }
                if let Some(id) = input.get("roleTemplateId").and_then(Value::as_str) {
                    let template = data.get("roles").and_then(|roles| roles.get(id)).cloned().or_else(|| ["ADMIN","EDITOR"].contains(&id).then(|| defaults(id))).ok_or("Role tidak ditemukan")?;
                    current.extend(template.as_object().ok_or("Role rusak")?.clone());
                    current.insert("roleTemplateId".into(),json!(id));
                    current.insert("moderationRole".into(),json!(if ["ADMIN","EDITOR"].contains(&id) {id} else {"CUSTOM"}));
                    current.insert("customRoleName".into(),json!(template["name"].as_str().unwrap_or_default()));
                }
                if let Some(value) = input.get("permissions") {current.insert("permissions".into(),permissions(current.get("permissions").unwrap_or(&Value::Null),value));}
                if let Some(value) = input.get("mutedUntil") {current.insert("mutedUntil".into(),json!(value.as_f64().filter(|n| n.is_finite()).unwrap_or_default().max(0.0)));}
                for field in ["bypassAppMute","bypassWindowsMute"] {if let Some(value) = input.get(field) {current.insert(field.into(),json!(value == true));}}
            }
            let policies = next.entry("profilePolicies").or_insert_with(|| json!({})).as_object_mut().ok_or("Pengaturan profil rusak")?;
            policies.insert(key.clone(),Value::Object(current));
            persist(&service.access,&mut data,next,"moderation-change",json!({"profile":key,"name":name}))?;
        }
        announce(&service);
        snapshot_value(&service)
    }.await;
    response(result)
}

async fn update_role(State(service): State<Service>, request: Request) -> Response {
    if !host(&service,request.headers()) { return denied() }
    let result = async {
        let input = input(request).await?;
        let id = input["id"].as_str().map(str::to_owned).unwrap_or_else(|| format!("role-{}",uuid::Uuid::new_v4()));
        let name: String = input["name"].as_str().unwrap_or_default().trim().chars().take(40).collect();
        if (!matches!(id.as_str(),"ADMIN"|"EDITOR") && !id.strip_prefix("role-").is_some_and(|tail| !tail.is_empty() && tail.len()<=80 && tail.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte==b'-')))
            || name.is_empty() || ["developer","host"].contains(&name.to_lowercase().as_str()) {return Err("Nama role tidak valid".into())}
        let users = service.users();
        {
            let mut data = service.access.data.lock().expect("moderation policy");
            if data.get("roles").and_then(Value::as_object).is_some_and(|roles| roles.iter().any(|(key,role)| key!=&id && crate::profiles::key(role["name"].as_str().unwrap_or_default()) == crate::profiles::key(&name))) {return Err("Nama role sudah ada".into())}
            let previous = data.get("roles").and_then(|roles| roles.get(&id)).cloned().unwrap_or_else(|| defaults(&id));
            let role = json!({"name":name,"permissions":permissions(&previous["permissions"],&input["permissions"]),
                "mentionLevel":level(input.get("mentionLevel").unwrap_or(&previous["mentionLevel"])),
                "fullscreenMode":mode(input.get("fullscreenMode").unwrap_or(&previous["fullscreenMode"])),
                "bypassAppMute":input.get("bypassAppMute").map(|value| value==true).unwrap_or(previous["bypassAppMute"]==true),
                "bypassWindowsMute":input.get("bypassWindowsMute").map(|value| value==true).unwrap_or(previous["bypassWindowsMute"]==true)});
            let mut next = data.clone();
            next.entry("roles").or_insert_with(|| json!({})).as_object_mut().ok_or("Daftar role rusak")?.insert(id.clone(),role.clone());
            let mut policies = data.get("profilePolicies").and_then(Value::as_object).cloned().unwrap_or_default();
            for user in users {
                let key = crate::profiles::key(user["name"].as_str().unwrap_or_default());
                let previous = policies.get(&key).or_else(|| data.get("users").and_then(|users| users.get(user["deviceId"].as_str().unwrap_or_default()))).cloned().unwrap_or_else(|| json!({}));
                if previous["roleTemplateId"].as_str().or_else(|| previous["moderationRole"].as_str()).unwrap_or("EDITOR") == id {
                    let mut policy = previous.as_object().cloned().unwrap_or_default();
                    policy.extend(role.as_object().unwrap().clone());
                    policy.insert("roleTemplateId".into(),json!(id));
                    policy.insert("customRoleName".into(),json!(name));
                    policies.insert(key,Value::Object(policy));
                }
            }
            next.insert("profilePolicies".into(),Value::Object(policies));
            persist(&service.access,&mut data,next,"role-change",json!({"id":id,"name":name}))?;
        }
        announce(&service);
        snapshot_value(&service)
    }.await;
    response(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_audit_prevents_partial_policy_change() {
        let directory = std::env::temp_dir().join(format!("kc-admin-audit-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("moderation.json");
        let old = json!({"profilePolicies":{"editor":{"permissions":{"sendChat":true}}}}).as_object().unwrap().clone();
        crate::host::atomic_save(&path,&old).unwrap();
        let access = Access::open(path.clone()).unwrap();
        std::fs::write(directory.join("audit-log.json"),b"{invalid audit").unwrap();
        let mut state = access.data.lock().unwrap();
        let mut next = old.clone();
        next.insert("tagSpamPolicy".into(),spam(&json!({"blockSeconds":10})));
        assert!(persist(&access,&mut state,next.clone(),"tag-spam-policy",json!({})).is_err());
        assert_eq!(*state,old);
        assert_eq!(serde_json::from_slice::<Map<String,Value>>(&std::fs::read(&path).unwrap()).unwrap(),old);
        std::fs::write(directory.join("audit-log.json"),b"[]").unwrap();
        persist(&access,&mut state,next.clone(),"tag-spam-policy",json!({})).unwrap();
        assert_eq!(*state,next);
        let audit: Vec<Value> = serde_json::from_slice(&std::fs::read(directory.join("audit-log.json")).unwrap()).unwrap();
        assert_eq!(audit.len(),1);
        assert_eq!(audit[0]["type"],"tag-spam-policy");
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
