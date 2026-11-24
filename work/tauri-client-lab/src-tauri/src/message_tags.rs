use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub struct Guard { buckets: Mutex<HashMap<String, Bucket>> }
#[derive(Default)]
struct Bucket { events: Vec<(u64, bool, bool)>, blocked_until: u64, last_at: u64 }

pub struct Tags { pub fields: Value, pub everyone: bool, pub mass: bool, pub has_mention: bool, pub work_tag: bool }

fn failure(error: &str) -> Value { json!({"ok":false,"error":error}) }

pub fn prepare(text: &str, raw: &Value, actor: &str, host: bool, known: &[String], conversation: &Value, policy: &Value, now: u64) -> Result<Tags, Value> {
    prepare_with_profiles(text,raw,actor,host,known,conversation,policy,now,&HashMap::new())
}

pub fn prepare_with_profiles(text: &str, raw: &Value, actor: &str, host: bool, known: &[String], conversation: &Value, policy: &Value, now: u64, aliases: &HashMap<String,String>) -> Result<Tags, Value> {
    let text_lower = text.to_lowercase();
    let mut everyone = false;
    let mut editor = false;
    let mut names = Vec::new();
    let mut handles = vec![("everyone".to_owned(), None), ("editor".to_owned(), None)];
    for name in known.iter().map(|name| name.trim()).filter(|name| !name.is_empty()) {
        handles.push((name.to_lowercase(), Some(name)));
    }
    for name in known.iter().map(|name| name.trim()).filter(|name| !name.is_empty()) {
        if let Some(alias) = aliases.get(&name.to_lowercase()).filter(|alias| !alias.trim().is_empty()) {
            handles.push((alias.to_lowercase(), Some(name)));
        }
    }
    handles.sort_by_key(|(handle, _)| std::cmp::Reverse(handle.len()));
    for (index, _) in text_lower.match_indices('@') {
        let tail = &text_lower[index + 1..];
        let Some((handle, target)) = handles.iter().find(|(handle, _)| {
            tail.strip_prefix(handle.as_str()).is_some_and(|rest|
                !rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_'))
        }) else { continue };
        if target.is_some() && handles.iter().any(|(candidate, other)| candidate == handle && other != target)
            && !known.iter().any(|name| name.trim().eq_ignore_ascii_case(handle) || name.trim().to_lowercase() == *handle) {
            return Err(failure("Nama tag dipakai beberapa orang. Pilih penerima dari daftar @ berdasarkan PC."));
        }
        match target {
            None if handle == "everyone" => everyone = true,
            None => editor = true,
            Some(name) if !names.contains(name) => names.push(*name),
            _ => {},
        }
    }
    let has_mention = everyone || editor || !names.is_empty();
    let permissions = &policy["permissions"];
    if has_mention {
        if policy["mutedUntil"].as_f64().unwrap_or_default() > now as f64 { return Err(failure("Mention kamu sedang diblokir oleh HOST.")) }
        if everyone && permissions["everyone"] != true { return Err(failure("Kamu tidak punya izin @everyone. Minta Host mengaktifkannya.")) }
        if editor && permissions["editor"] != true { return Err(failure("Kamu tidak punya izin @editor. Minta Host mengaktifkannya.")) }
        if !names.is_empty() && permissions["personal"] != true { return Err(failure("Izin tag nama kamu sedang dinonaktifkan oleh Host.")) }
        if names.len() > 5 && !host { return Err(failure("Maksimal 5 mention nama dalam satu pesan.")) }
    }
    let private = conversation["type"] == "dm";
    if private && (everyone || editor) { return Err(failure("@everyone dan @editor hanya untuk grup. Di chat pribadi gunakan tag nama penerima.")) }
    let allowed: Vec<_> = if conversation["type"] == "general" { known.iter().map(String::as_str).collect() }
        else { conversation["members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect() };
    let allowed: Vec<_> = allowed.into_iter().filter(|name| crate::profiles::key(name) != crate::profiles::key(actor)).collect();
    if names.iter().any(|name| !allowed.iter().any(|allowed| crate::profiles::key(allowed) == crate::profiles::key(name))) {
        return Err(failure(if private {"Di chat pribadi hanya bisa tag lawan chat ini."} else {"Tag nama hanya bisa untuk anggota grup ini."}));
    }
    let work_tag = raw["workTag"].as_str().filter(|tag| ["print-test","design-revision","design-request"].contains(tag));
    if let Some(tag) = work_tag {
        let permission = match tag { "print-test" => "tagPrintTest", "design-revision" => "tagDesignRevision", _ => "tagDesignRequest" };
        if permissions[permission] == false || permissions["banned"] == true {
            return Err(json!({"ok":false,"code":"FEATURE_DENIED","error":"Tag pekerjaan ini dibatasi HOST"}));
        }
    }
    let mut target = Value::Null;
    if work_tag.is_some() {
        if private {
            if let Some(peer) = allowed.first() { target = json!({"type":"user","name":peer}); }
        } else if raw["workTagTarget"]["type"] == "all" { target = json!({"type":"all"}); }
        else if raw["workTagTarget"]["type"] == "user" {
            let name: String = raw["workTagTarget"]["name"].as_str().unwrap_or_default().trim().chars().take(40).collect();
            if !allowed.iter().any(|allowed| crate::profiles::key(allowed) == crate::profiles::key(&name)) {
                return Err(failure("Target tag harus anggota percakapan ini."));
            }
            target = json!({"type":"user","name":name});
        }
    }
    let tagged = has_mention || work_tag.is_some();
    let video = tagged && policy["tagVideoEnabled"] == true;
    Ok(Tags { everyone, mass: everyone || editor, has_mention, work_tag: work_tag.is_some(), fields: json!({
        "workTag":work_tag,"workTagTarget":target,"mentionRecipients":names,"mentionMass":everyone||editor,"tagScope":if tagged {Some(if private {"private"} else {"group"})} else {None},
        "tagVideoEnabled":video,"mentionLevel":if !tagged {"standard"} else if video {"fullscreen"} else {policy["mentionLevel"].as_str().unwrap_or("notice")},
        "fullscreenMode":if video {"video"} else {policy["fullscreenMode"].as_str().unwrap_or("video")},"bledarVideo":null,
        "bypassAppMute":tagged && policy["bypassAppMute"] == true,"bypassWindowsMute":tagged && policy["bypassWindowsMute"] == true,
        "moderationRole":policy["roleLabel"].as_str().unwrap_or("Editor")
    }) })
}

#[test]
fn duplicate_display_names_require_a_specific_account() {
    let known = vec!["Iqbal A".to_owned(), "Iqbal B".to_owned()];
    let aliases = HashMap::from([
        ("iqbal a".into(), "Iqbal".into()),
        ("iqbal b".into(), "Iqbal".into()),
    ]);
    let policy = json!({"permissions":{"personal":true}});
    let conversation = json!({"type":"general"});
    let prepare = |text| prepare_with_profiles(
        text, &Value::Null, "Agus", false, &known, &conversation, &policy, 0, &aliases,
    );
    assert!(prepare("@Iqbal").is_err());
    assert_eq!(prepare("@Iqbal B").unwrap().fields["mentionRecipients"], json!(["Iqbal B"]));
}

impl Guard {
    pub fn check(&self, device: &str, host: bool, tags: &Tags, policy: &Value, now: u64) -> Result<(), Value> {
        if host || (!tags.has_mention && !tags.work_tag) { return Ok(()) }
        let clamp = |key: &str, min: u64, max: u64, default| policy.get(key).and_then(|value| match value {
            Value::Null => Some(0.0), Value::Bool(flag) => Some(if *flag {1.0} else {0.0}),
            Value::String(text) => if text.trim().is_empty() {Some(0.0)} else {text.trim().parse().ok()},
            _ => value.as_f64(),
        }).filter(|n| n.is_finite())
            .map(|n| n.round().max(min as f64).min(max as f64) as u64).unwrap_or(default);
        let burst_max = clamp("maxPer10Seconds", 1, 10, 2);
        let minute_max = clamp("maxPerMinute", 2, 30, 6);
        let seconds = clamp("blockSeconds", 5, 300, 30);
        let mut buckets = self.buckets.lock().expect("tag notification guard");
        if buckets.len() > 4096 { buckets.retain(|_,bucket| now.saturating_sub(bucket.last_at) <= 120000); }
        let bucket = buckets.entry(device.into()).or_default();
        if bucket.blocked_until > now { return Err(json!({"ok":false,"error":format!("Perlindungan spam tag aktif. Coba lagi dalam {} detik.", (bucket.blocked_until-now).div_ceil(1000)),"retryAfterMs":bucket.blocked_until-now})) }
        bucket.events.retain(|(at,_,_)| now.saturating_sub(*at) < 60000);
        let burst = bucket.events.iter().filter(|(at,_,_)| now.saturating_sub(*at) < 10000).count() as u64;
        let tag_burst = bucket.events.iter().filter(|(at,tag,_)| *tag && now.saturating_sub(*at) < 10000).count() as u64;
        let mass_minute = bucket.events.iter().filter(|(_,_,mass)| *mass).count();
        let mass_burst = bucket.events.iter().filter(|(at,_,mass)| *mass && now.saturating_sub(*at) < 20000).count();
        let blocked = (tags.work_tag && tag_burst >= burst_max) || (tags.mass && (mass_minute >= 3 || mass_burst >= 1))
            || bucket.events.len() as u64 >= minute_max || burst >= 3.max(burst_max+1);
        bucket.last_at = now;
        if blocked {
            bucket.blocked_until = now + seconds*1000;
            return Err(json!({"ok":false,"error":format!("Tag terlalu sering dikirim. Tag dinonaktifkan sementara selama {seconds} detik."),"retryAfterMs":seconds*1000}));
        }
        bucket.events.push((now,tags.work_tag,tags.mass));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn throttle_matches_original_for_bursts_mass_tags_and_recovery() {
        let fixtures: Value = serde_json::from_str(include_str!("../../message-tags-fixtures.json")).unwrap();
        for sequence in fixtures["sequences"].as_array().unwrap() {
            let guard = Guard::default();
            for event in sequence["events"].as_array().unwrap() {
                let tags = Tags { fields: Value::Null, everyone: event["mass"] == true, mass: event["mass"] == true,
                    work_tag: event["tag"] == true, has_mention: event["mass"] == true || !event["names"].as_array().unwrap().is_empty() };
                let result = guard.check("pc1",false,&tags,&sequence["policy"],1000000+event["at"].as_u64().unwrap());
                assert_eq!(result.err().unwrap_or(Value::Null),event["expected"]);
            }
        }
    }
}
