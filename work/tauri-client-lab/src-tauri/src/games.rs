use crate::{chess, profiles::Profiles};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub const EVENTS: &[&str] = &[
    "game-create",
    "game-join",
    "game-decline",
    "game-add-bot",
    "game-state-request",
    "game-active-request",
    "game-restart-request",
    "game-cancel",
    "game-leave",
    "game-open-slither-list",
    "chess-move",
    "snakes-roll",
    "ludo-roll",
    "ludo-move",
];
#[derive(Clone)]
struct Room {
    data: Value,
    solo_at: u64,
    countdown_at: u64,
    bot_at: u64,
    turn_at: u64,
    automatic: Option<(u64, usize, u64, bool)>,
    recent: VecDeque<(String, Value)>,
}
impl Room {
    fn new(data: Value) -> Self {
        Self {
            data,
            solo_at: 0,
            countdown_at: 0,
            bot_at: 0,
            turn_at: 0,
            automatic: None,
            recent: VecDeque::new(),
        }
    }

    fn deadline(&self) -> Option<u64> {
        [
            self.solo_at,
            self.countdown_at,
            self.bot_at,
            self.turn_at,
            self.automatic.map_or(0, |(deadline, _, _, _)| deadline),
        ]
        .into_iter()
        .filter(|deadline| *deadline > 0)
        .min()
    }
}
pub struct Event {
    pub names: Vec<String>,
    pub event: &'static str,
    pub payload: Value,
}
pub struct Server {
    path: PathBuf,
    rooms: Mutex<Vec<Room>>,
    profiles: Arc<Profiles>,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..10].into()
}
fn random(max: usize) -> usize {
    (uuid::Uuid::new_v4().as_u128() % max as u128) as usize
}
fn key(name: &str) -> String {
    crate::profiles::key(name)
}
fn player(g: &Value, name: &str) -> Option<usize> {
    g["players"]
        .as_array()?
        .iter()
        .position(|p| key(p["name"].as_str().unwrap_or_default()) == key(name))
}
fn fail(error: &str) -> Value {
    json!({"ok":false,"error":error})
}
fn names(g: &Value) -> Vec<String> {
    g["players"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["isBot"] != true)
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .chain(
            g["invitedNames"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned)),
        )
        .collect()
}
fn bump(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) {
    room.data["updatedAt"] = json!(time);
    room.data["revision"] = json!(room.data["revision"].as_u64().unwrap_or_default() + 1);
    events.push(Event {
        names: names(&room.data),
        event: "game-state",
        payload: public(&room.data, profiles),
    });
}
fn stop(room: &mut Room) {
    room.solo_at = 0;
    room.countdown_at = 0;
    room.bot_at = 0;
    room.turn_at = 0;
    room.automatic = None;
}
fn schedule(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) {
    room.bot_at = 0;
    room.turn_at = 0;
    if room.data["status"] != "playing" {
        room.data["turnDeadline"] = json!(0);
        return;
    }
    room.data["turnDuration"] = json!(30);
    room.data["turnDeadline"] = json!(time + 30000);
    let turn = room.data["turn"].as_u64().unwrap_or_default() as usize;
    if room.data["players"][turn]["isBot"] == true {
        room.bot_at = time + 700;
    } else {
        room.turn_at = time + 30000;
    }
    bump(room, events, profiles, time);
}
const COLORS: [&str; 4] = ["red", "green", "yellow", "blue"];
fn colors(g: &mut Value) {
    let mut used = Vec::new();
    for p in g["players"].as_array_mut().expect("validated players") {
        let want = p["color"].as_str().unwrap_or_default().to_lowercase();
        let c = if COLORS.contains(&want.as_str()) && !used.contains(&want) {
            want
        } else {
            COLORS
                .iter()
                .find(|c| !used.iter().any(|used| used == **c))
                .unwrap_or(&"red")
                .to_string()
        };
        p["color"] = json!(c);
        used.push(c);
    }
}
fn snakes_board() -> Value {
    json!({"snakes":{"16":6,"48":26,"49":11,"56":53,"62":19,"64":60,"87":24,"93":73,"95":75,"98":78},
    "ladders":{"1":38,"4":14,"9":31,"21":42,"28":84,"36":44,"51":67,"71":91,"80":100}})
}
fn start(g: &mut Value, reverse: bool) {
    if g["type"] == "chess" {
        if reverse {
            g["players"].as_array_mut().unwrap().reverse();
        }
        g["players"][0]["color"] = json!("white");
        if g["players"].as_array().unwrap().len() > 1 {
            g["players"][1]["color"] = json!("black");
        }
        g["board"] = chess::value(&chess::initial());
        g["castlingRights"] = chess::rights();
        g["enPassantTarget"] = json!(-1);
        g["draw"] = json!(false);
    } else if g["type"] == "snakes" {
        g["snakesLadders"] = json!({"positions":vec![0;g["players"].as_array().unwrap().len()],"board":snakes_board()});
    }
    g["status"] = json!("playing");
    g["turn"] = json!(0);
    g["dice"] = json!(0);
    g["winner"] = Value::Null;
    g["soloCountdown"] = json!(0);
    g["idleMisses"] = json!({});
}
fn start_chess(room: &mut Room) {
    let pref = room.data["players"][0]["color"]
        .as_str()
        .unwrap_or("random");
    let reverse = pref == "black" || (pref == "random" && random(2) == 0);
    start(&mut room.data, reverse);
}
fn solo(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) {
    if room.solo_at == 0
        && room.data["status"] == "lobby"
        && room.data["players"].as_array().unwrap().len() == 1
        && room.data["invitedNames"]
            .as_array()
            .is_none_or(Vec::is_empty)
    {
        room.data["soloCountdown"] = json!(10);
        room.solo_at = time + 1000;
        bump(room, events, profiles, time);
    }
}
fn countdown(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) {
    if room.data["type"] == "ludo"
        && room.data["status"] == "lobby"
        && room.data["players"].as_array().unwrap().len() >= 2
        && room.countdown_at == 0
    {
        room.solo_at = 0;
        room.data["soloCountdown"] = json!(0);
        room.data["countdown"] = json!(10);
        room.countdown_at = time + 1000;
        bump(room, events, profiles, time);
    }
}
fn add_bot(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) -> bool {
    let max = if room.data["type"] == "ludo" { 4 } else { 2 };
    if room.data["status"] != "lobby" || room.data["players"].as_array().unwrap().len() >= max {
        return false;
    }
    let players = room.data["players"].as_array_mut().unwrap();
    let name = ["Bot Ceria", "Bot Biru", "Bot Kuning"]
        .into_iter()
        .find(|name| !players.iter().any(|p| p["name"] == *name))
        .unwrap_or("Bot");
    players.push(json!({"name":name,"isBot":true,"color":null}));
    if room.data["type"] == "chess" {
        start_chess(room)
    } else if room.data["type"] == "snakes" {
        start(&mut room.data, false)
    } else {
        colors(&mut room.data);
        countdown(room, events, profiles, time);
    }
    bump(room, events, profiles, time);
    if room.data["status"] == "playing" {
        schedule(room, events, profiles, time);
    }
    true
}
fn public(g: &Value, profiles: &Profiles) -> Value {
    let board = chess::board(&g["board"]);
    let ci = board
        .as_ref()
        .map(chess::check_info)
        .unwrap_or_else(|| json!({"player":-1,"king":-1,"checkers":[]}));
    let turn = g["turn"].as_u64().unwrap_or_default() as usize;
    let legal = if g["type"] == "chess" && g["status"] == "playing" {
        board
            .as_ref()
            .map(|b| {
                chess::moves(
                    b,
                    turn as i32,
                    &g["castlingRights"],
                    g["enPassantTarget"].as_i64().unwrap_or(-1),
                )
            })
            .unwrap_or_default()
    } else {
        vec![]
    };
    let players:Vec<_>=g["players"].as_array().into_iter().flatten().map(|p|json!({"name":p["name"],"isBot":p["isBot"]==true,"color":p["color"],
        "avatarUrl":if p["isBot"]==true{Value::Null}else{profiles.get(p["name"].as_str().unwrap_or_default())["avatarUrl"].clone()}})).collect();
    let mut result = json!({"players":players,"board":g["board"],"winner":g["winner"],"lastAction":g["lastAction"],"ludo":g["ludo"],"snakesLadders":g["snakesLadders"],"slither":null,
        "checkPlayer":ci["player"],"checkedKingIndex":ci["king"],"checkerIndices":ci["checkers"],"legalMoves":legal,
        "legalPawns":if g["type"]=="ludo"&&g["status"]=="playing"&&g["dice"].as_u64().unwrap_or_default()>0{ludo_legal(g,turn,g["dice"].as_u64().unwrap())}else{vec![]}});
    for field in [
        "id",
        "type",
        "host",
        "status",
        "botLevel",
        "invitedNames",
        "declinedNames",
    ] {
        result[field] = g[field].clone();
    }
    for field in [
        "revision",
        "countdown",
        "soloCountdown",
        "turn",
        "dice",
        "turnDeadline",
    ] {
        result[field] = json!(g[field].as_u64().unwrap_or_default());
    }
    result["turnDuration"] = json!(g["turnDuration"].as_u64().filter(|n| *n > 0).unwrap_or(30));
    result
}
fn ludo_legal(g: &Value, pi: usize, d: u64) -> Vec<usize> {
    (0..4)
        .filter(|pawn| {
            g["ludo"]["pawns"][pi][*pawn].as_i64().is_some_and(|pos| {
                if pos < 0 {
                    d == 6
                } else {
                    pos + d as i64 <= 57
                }
            })
        })
        .collect()
}
fn reset_idle(g: &mut Value, pi: usize) {
    let k = key(g["players"][pi]["name"].as_str().unwrap_or_default());
    if !g["idleMisses"].is_object() {
        g["idleMisses"] = json!({});
    }
    g["idleMisses"][k] = json!(0);
}
fn ludo_move(
    room: &mut Room,
    pi: usize,
    pawn: usize,
    d: u64,
    events: &mut Vec<Event>,
    profiles: &Profiles,
    time: u64,
) {
    let g = &mut room.data;
    colors(g);
    let from = g["ludo"]["pawns"][pi][pawn].as_i64().unwrap();
    let pos = if from < 0 { 0 } else { from + d as i64 };
    g["ludo"]["pawns"][pi][pawn] = json!(pos);
    let mut captured = Vec::new();
    let count = g["players"].as_array().unwrap().len();
    if (0..52).contains(&pos) {
        let offset = |g: &Value, i: usize| {
            COLORS
                .iter()
                .position(|c| g["players"][i]["color"] == *c)
                .unwrap_or(i)
                * 13
        };
        let global = (pos + offset(g, pi) as i64) % 52;
        for op in 0..count {
            if op == pi {
                continue;
            }
            for k in 0..4 {
                let q = g["ludo"]["pawns"][op][k].as_i64().unwrap();
                if (0..52).contains(&q) && (q + offset(g, op) as i64) % 52 == global {
                    captured.push(json!({"player":op,"pawn":k,"from":q}));
                    g["ludo"]["pawns"][op][k] = json!(-1);
                }
            }
        }
    }
    g["lastAction"] = json!({"id":id(),"type":"ludo-move","player":pi,"pawn":pawn,"from":from,"to":pos,"dice":d,"captured":captured,"ts":time});
    if g["ludo"]["pawns"][pi]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p == 57)
    {
        g["status"] = json!("finished");
        g["winner"] = g["players"][pi]["name"].clone();
    } else if d != 6 {
        g["turn"] = json!((pi + 1) % count);
    }
    g["dice"] = json!(0);
    bump(room, events, profiles, time);
    schedule(room, events, profiles, time);
}
fn snakes_roll(
    room: &mut Room,
    pi: usize,
    d: u64,
    events: &mut Vec<Event>,
    profiles: &Profiles,
    time: u64,
) -> Value {
    let g = &mut room.data;
    let from = g["snakesLadders"]["positions"][pi].as_u64().unwrap();
    let mut to = from + d;
    if to > 100 {
        to = from
    } else {
        let board = snakes_board();
        to = board["ladders"][to.to_string()]
            .as_u64()
            .or_else(|| board["snakes"][to.to_string()].as_u64())
            .unwrap_or(to);
    }
    g["dice"] = json!(d);
    g["snakesLadders"]["positions"][pi] = json!(to);
    g["lastAction"] =
        json!({"id":id(),"type":"snakes-roll","player":pi,"dice":d,"from":from,"to":to,"ts":time});
    if to == 100 {
        g["status"] = json!("finished");
        g["winner"] = g["players"][pi]["name"].clone();
    } else {
        g["turn"] = json!((pi + 1) % g["players"].as_array().unwrap().len());
    }
    bump(room, events, profiles, time);
    if room.data["status"] == "playing" {
        schedule(room, events, profiles, time);
    }
    json!({"ok":true,"dice":d,"from":from,"to":to,"revision":room.data["revision"]})
}
fn bot(room: &mut Room, events: &mut Vec<Event>, profiles: &Profiles, time: u64) {
    let pi = room.data["turn"].as_u64().unwrap_or_default() as usize;
    if room.data["type"] == "chess" {
        if let Some([from, to]) = chess::bot_move(&room.data, pi as i32) {
            chess::apply(&mut room.data, from, to, pi, &id(), time);
            bump(room, events, profiles, time);
            schedule(room, events, profiles, time);
        } else {
            let b = chess::board(&room.data["board"]).unwrap();
            room.data["status"] = json!("finished");
            room.data["winner"] = if chess::in_check(&b, pi as i32) {
                room.data["players"][1 - pi]["name"].clone()
            } else {
                Value::Null
            };
            bump(room, events, profiles, time);
        }
    } else if room.data["type"] == "snakes" {
        snakes_roll(room, pi, 1 + random(6) as u64, events, profiles, time);
    } else {
        let d = 1 + random(6) as u64;
        room.data["dice"] = json!(d);
        room.data["lastAction"] = json!({"id":id(),"type":"ludo-roll","player":pi,"dice":d,"power":35+random(61),"ts":time});
        room.automatic = Some((time + 650, pi, d, true));
        bump(room, events, profiles, time);
    }
}
fn validate(g: &Value) -> bool {
    let Some(players) = g["players"].as_array() else {
        return false;
    };
    if players.is_empty()
        || players.len() > 4
        || players.iter().any(|p| p["name"].as_str().is_none())
        || g["id"].as_str().is_none()
        || g["host"].as_str().is_none()
    {
        return false;
    }
    if !["lobby", "playing"].iter().any(|s| g["status"] == *s)
        || g["turn"].as_u64().unwrap_or_default() >= players.len() as u64
    {
        return false;
    }
    match g["type"].as_str() {
        Some("chess") => players.len() <= 2 && chess::board(&g["board"]).is_some(),
        Some("snakes") => {
            players.len() <= 2
                && g["snakesLadders"]["positions"].as_array().is_some_and(|v| {
                    v.len() >= players.len()
                        && v.iter().all(|v| v.as_u64().is_some_and(|v| v <= 100))
                })
        }
        Some("ludo") => g["ludo"]["pawns"].as_array().is_some_and(|rows| {
            rows.len() >= players.len()
                && rows.iter().all(|row| {
                    row.as_array().is_some_and(|row| {
                        row.len() == 4
                            && row
                                .iter()
                                .all(|p| p.as_i64().is_some_and(|p| (-1..=57).contains(&p)))
                    })
                })
        }),
        _ => false,
    }
}
impl Server {
    pub fn open(path: PathBuf, profiles: Arc<Profiles>) -> io::Result<Self> {
        let saved: Vec<Value> = match std::fs::File::open(&path) {
            Ok(file) => serde_json::from_reader(file).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let time = now();
        let mut rooms = Vec::new();
        let mut ignored = Vec::new();
        for data in saved {
            if data["type"] == "slither"
                || data["status"] == "finished"
                || data["savedAt"]
                    .as_u64()
                    .is_none_or(|at| at == 0 || time.saturating_sub(at) >= 43200000)
            {
                continue;
            }
            if !validate(&data) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Data pertandingan lama tidak valid; berkas tidak direset.",
                ));
            }
            let mut room = Room::new(data);
            if room.data["type"] == "ludo" {
                colors(&mut room.data);
            }
            if room.data["status"] == "playing" {
                schedule(&mut room, &mut ignored, &profiles, time);
            } else if room.data["type"] == "ludo"
                && room.data["players"].as_array().unwrap().len() >= 2
            {
                countdown(&mut room, &mut ignored, &profiles, time);
            } else {
                solo(&mut room, &mut ignored, &profiles, time);
            }
            rooms.push(room);
        }
        let server = Self {
            path,
            rooms: Mutex::new(rooms),
            profiles,
        };
        if !ignored.is_empty() {
            server.save(&server.rooms.lock().unwrap(), time)?;
        }
        Ok(server)
    }
    fn save(&self, rooms: &[Room], time: u64) -> io::Result<()> {
        let rows: Vec<_> = rooms
            .iter()
            .filter(|r| r.data["status"] != "finished")
            .map(|r| {
                let mut data = r.data.clone();
                data["savedAt"] = json!(time);
                if let Some(map) = data.as_object_mut() {
                    for key in [
                        "timer",
                        "soloTimer",
                        "botTimer",
                        "turnTimer",
                        "slitherTimer",
                        "_recentClientActions",
                    ] {
                        map.remove(key);
                    }
                }
                data
            })
            .collect();
        crate::host::atomic_write(&self.path, |file| {
            use std::io::Write;
            file.write_all(&serde_json::to_vec_pretty(&rows)?)
        })
    }
    pub fn handle(
        &self,
        event: &str,
        actor: &str,
        input: &Value,
        online: &[String],
    ) -> (Value, Vec<Event>) {
        let time = now();
        let mut rooms = self.rooms.lock().expect("game rooms");
        let mut next = rooms.clone();
        let mut events = Vec::new();
        let reply = action(
            &mut next,
            event,
            actor,
            input,
            online,
            &self.profiles,
            &mut events,
            time,
        );
        if reply["ok"] == false {
            return (reply, vec![]);
        }
        if !events.is_empty() {
            if let Err(error) = self.save(&next, time) {
                return (
                    json!({"ok":false,"retryable":true,"error":format!("Pertandingan belum tersimpan: {error}")}),
                    vec![],
                );
            }
        }
        *rooms = next;
        (reply, events)
    }
    pub fn reconnect(&self, actor: &str) -> Vec<(&'static str, Value)> {
        let rooms = self.rooms.lock().expect("game rooms");
        let active: Vec<_> = rooms
            .iter()
            .filter(|room| room.data["status"] != "finished" && player(&room.data, actor).is_some())
            .map(|room| public(&room.data, &self.profiles))
            .collect();
        let mut events = Vec::new();
        if !active.is_empty() {
            events.push(("game-reconnect", json!(active)));
        }
        events.push(("game-open-slither-list", json!([])));
        for room in rooms
            .iter()
            .filter(|room| room.data["status"] == "lobby" && player(&room.data, actor).is_none())
        {
            let contains = |field: &str| {
                room.data[field]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|name| key(name.as_str().unwrap_or_default()) == key(actor))
            };
            if contains("invitedNames") && !contains("declinedNames") {
                events.push(("game-invite", json!({"id":room.data["id"],"type":room.data["type"],"host":room.data["host"],"target":actor,"invitees":room.data["invitedNames"]})));
            }
        }
        events
    }
    pub fn timer_delay(&self) -> Option<std::time::Duration> {
        let deadline = self
            .rooms
            .lock()
            .expect("game rooms")
            .iter()
            .filter_map(Room::deadline)
            .min()?;
        Some(std::time::Duration::from_millis(
            deadline.saturating_sub(now()),
        ))
    }

    pub fn tick(&self) -> Result<Vec<Event>, String> {
        self.tick_at(now())
    }

    fn tick_at(&self, time: u64) -> Result<Vec<Event>, String> {
        let mut rooms = self.rooms.lock().expect("game rooms");
        if !rooms
            .iter()
            .any(|room| room.deadline().is_some_and(|deadline| deadline <= time))
        {
            return Ok(Vec::new());
        }
        let mut next = rooms.clone();
        let mut events = Vec::new();
        let mut closed = Vec::new();
        for room in &mut next {
            if room.solo_at > 0 && time >= room.solo_at {
                room.solo_at = time + 1000;
                let count = room.data["soloCountdown"]
                    .as_u64()
                    .unwrap_or(1)
                    .saturating_sub(1);
                room.data["soloCountdown"] = json!(count);
                if count == 0 {
                    room.solo_at = 0;
                    add_bot(room, &mut events, &self.profiles, time);
                } else {
                    bump(room, &mut events, &self.profiles, time);
                }
            }
            if room.countdown_at > 0 && time >= room.countdown_at {
                room.countdown_at = time + 1000;
                let count = room.data["countdown"]
                    .as_u64()
                    .unwrap_or(1)
                    .saturating_sub(1);
                room.data["countdown"] = json!(count);
                if count == 0 {
                    room.countdown_at = 0;
                    start(&mut room.data, false);
                }
                bump(room, &mut events, &self.profiles, time);
                if count == 0 {
                    schedule(room, &mut events, &self.profiles, time);
                }
            }
            if let Some((at, pi, d, bot)) = room.automatic.filter(|(at, _, _, _)| time >= *at) {
                let _ = at;
                room.automatic = None;
                if room.data["status"] == "playing"
                    && room.data["turn"] == pi
                    && room.data["dice"] == d
                {
                    let legal = ludo_legal(&room.data, pi, d);
                    if bot && !legal.is_empty() {
                        let pawn = legal[random(legal.len())];
                        ludo_move(room, pi, pawn, d, &mut events, &self.profiles, time);
                    } else {
                        room.data["dice"] = json!(0);
                        if d != 6 {
                            room.data["turn"] =
                                json!((pi + 1) % room.data["players"].as_array().unwrap().len());
                        }
                        bump(room, &mut events, &self.profiles, time);
                        schedule(room, &mut events, &self.profiles, time);
                    }
                }
            }
            if room.bot_at > 0 && time >= room.bot_at {
                room.bot_at = 0;
                if room.data["status"] == "playing" {
                    bot(room, &mut events, &self.profiles, time);
                }
            }
            if room.turn_at > 0 && time >= room.turn_at {
                room.turn_at = 0;
                if room.data["status"] != "playing" {
                    continue;
                }
                let pi = room.data["turn"].as_u64().unwrap() as usize;
                let name = room.data["players"][pi]["name"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let k = key(&name);
                if !room.data["idleMisses"].is_object() {
                    room.data["idleMisses"] = json!({});
                }
                let misses = room.data["idleMisses"][&k].as_u64().unwrap_or_default() + 1;
                room.data["idleMisses"][k] = json!(misses);
                if misses >= 3
                    && room.data["players"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|p| p["isBot"] != true)
                {
                    events.push(Event {
                        names: names(&room.data),
                        event: "game-closed",
                        payload: json!({"id":room.data["id"],"reason":"idle-3x","by":name}),
                    });
                    closed.push(room.data["id"].clone());
                    continue;
                }
                room.data["lastAction"] =
                    json!({"id":id(),"type":"turn-assist","player":pi,"misses":misses,"ts":time});
                let d = room.data["dice"].as_u64().unwrap_or_default();
                if room.data["type"] == "ludo" && d > 0 {
                    let legal = ludo_legal(&room.data, pi, d);
                    if !legal.is_empty() {
                        ludo_move(
                            room,
                            pi,
                            legal[random(legal.len())],
                            d,
                            &mut events,
                            &self.profiles,
                            time,
                        );
                    } else {
                        room.data["dice"] = json!(0);
                        if d != 6 {
                            room.data["turn"] =
                                json!((pi + 1) % room.data["players"].as_array().unwrap().len());
                        }
                        bump(room, &mut events, &self.profiles, time);
                        schedule(room, &mut events, &self.profiles, time);
                    }
                } else {
                    bot(room, &mut events, &self.profiles, time);
                }
            }
        }
        next.retain(|room| !closed.contains(&room.data["id"]));
        if !events.is_empty() {
            self.save(&next, time).map_err(|e| e.to_string())?;
        }
        *rooms = next;
        Ok(events)
    }
}

fn action(
    rooms: &mut Vec<Room>,
    event: &str,
    actor: &str,
    input: &Value,
    online: &[String],
    profiles: &Profiles,
    events: &mut Vec<Event>,
    time: u64,
) -> Value {
    if event == "game-open-slither-list" {
        return json!({"ok":true,"rooms":[]});
    }
    if event == "game-active-request" {
        let state = rooms
            .iter()
            .filter(|r| r.data["status"] != "finished" && player(&r.data, actor).is_some())
            .max_by_key(|r| r.data["updatedAt"].as_u64().unwrap_or_default())
            .map(|r| public(&r.data, profiles));
        return json!({"ok":true,"state":state});
    }
    if event == "game-create" {
        if input["type"] == "slither" {
            return fail("Slither sudah dihapus dari aplikasi.");
        }
        let kind = match input["type"].as_str() {
            Some("ludo") => "ludo",
            Some("snakes") => "snakes",
            _ => "chess",
        };
        let mut invitees = Vec::new();
        for name in input["invitees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty() && key(n) != key(actor))
        {
            if invitees.iter().any(|n: &String| n == name) {
                continue;
            }
            if invitees.len() >= if kind == "ludo" { 3 } else { 1 } {
                break;
            }
            let Some(actual) = online.iter().find(|actual| key(actual) == key(name)) else {
                return fail(&format!("Editor tidak online: {name}"));
            };
            invitees.push(actual.clone());
        }
        let pref = input["color"].as_str().unwrap_or("random");
        let color = if kind == "ludo" {
            if COLORS.contains(&pref) {
                pref
            } else {
                ["red", "yellow", "green", "blue"][random(4)]
            }
        } else {
            pref
        };
        let mut g = json!({"id":id(),"type":kind,"host":actor,"players":[{"name":actor,"isBot":false,"color":color}],"invitedNames":invitees,"declinedNames":[],
            "botLevel":input["botLevel"].as_str().filter(|v|["easy","medium","hard"].contains(v)).unwrap_or("medium"),"status":"lobby","countdown":0,"soloCountdown":0,"turn":0,"winner":null,"dice":0,"lastAction":null,"turnDeadline":0,"turnDuration":30});
        if kind == "chess" {
            g["board"] = chess::value(&chess::initial());
        } else if kind == "snakes" {
            g["snakesLadders"] = json!({"positions":[0,0],"board":snakes_board()});
        } else {
            g["ludo"] = json!({"pawns":[[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1]]});
        }
        let mut room = Room::new(g);
        bump(&mut room, events, profiles, time);
        if invitees.is_empty() {
            solo(&mut room, events, profiles, time);
        } else {
            for target in &invitees {
                events.push(Event{names:vec![target.clone()],event:"game-invite",payload:json!({"id":room.data["id"],"type":kind,"host":actor,"target":target,"invitees":invitees})});
            }
        }
        let reply = json!({"ok":true,"id":room.data["id"],"state":public(&room.data,profiles)});
        rooms.push(room);
        return reply;
    }
    let Some(index) = rooms.iter().position(|r| r.data["id"] == input["id"]) else {
        return fail("Game tidak ditemukan");
    };
    let room = &mut rooms[index];
    let pi = player(&room.data, actor);
    if event == "game-cancel" || event == "game-leave" {
        if event == "game-cancel" && key(actor) != key(room.data["host"].as_str().unwrap()) {
            return fail("Hanya pembuat game yang bisa membatalkan");
        }
        if event == "game-leave" && pi.is_none() {
            return fail("Pemain tidak terdaftar");
        }
        events.push(Event{names:names(&room.data),event:"game-closed",payload:json!({"id":room.data["id"],"reason":if event=="game-cancel"{"host-cancel"}else{"player-left"},"by":actor})});
        rooms.remove(index);
        return json!({"ok":true});
    }
    if event == "game-join" {
        if room.data["status"] != "lobby" {
            return fail("Game sudah dimulai");
        }
        if pi.is_some() {
            return json!({"ok":true,"state":public(&room.data,profiles)});
        }
        let invited = room.data["invitedNames"].as_array().unwrap();
        if !invited.is_empty()
            && !invited
                .iter()
                .any(|v| key(v.as_str().unwrap_or_default()) == key(actor))
        {
            return fail("Game ini hanya untuk editor yang diundang");
        }
        if room.data["players"].as_array().unwrap().len()
            >= if room.data["type"] == "ludo" { 4 } else { 2 }
        {
            return fail("Room sudah penuh");
        }
        room.solo_at = 0;
        room.data["soloCountdown"] = json!(0);
        room.data["declinedNames"]
            .as_array_mut()
            .unwrap()
            .retain(|v| key(v.as_str().unwrap_or_default()) != key(actor));
        room.data["players"].as_array_mut().unwrap().push(
            json!({"name":actor,"isBot":false,"color":input["color"].as_str().unwrap_or("random")}),
        );
        if room.data["type"] == "chess" {
            start_chess(room);
        } else if room.data["type"] == "snakes" {
            start(&mut room.data, false);
        } else {
            colors(&mut room.data);
            countdown(room, events, profiles, time);
        }
        bump(room, events, profiles, time);
        schedule(room, events, profiles, time);
        return json!({"ok":true,"state":public(&room.data,profiles)});
    }
    if event == "game-decline" {
        if room.data["status"] != "lobby" {
            return fail("Lobby tidak aktif");
        }
        if !room.data["invitedNames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| key(n.as_str().unwrap_or_default()) == key(actor))
        {
            return fail("Kamu tidak termasuk undangan");
        }
        let declined = room.data["declinedNames"].as_array_mut().unwrap();
        if !declined.iter().any(|n| n == actor) {
            declined.push(json!(actor));
        }
        bump(room, events, profiles, time);
        return json!({"ok":true});
    }
    if event == "game-add-bot" {
        if key(actor) != key(room.data["host"].as_str().unwrap()) {
            return fail("Hanya pembuat game yang bisa menambah bot");
        }
        if !add_bot(room, events, profiles, time) {
            return fail("Bot tidak bisa ditambahkan");
        }
        return json!({"ok":true,"state":public(&room.data,profiles)});
    }
    let Some(pi) = pi else {
        return fail("Pertandingan sudah tidak aktif");
    };
    if event == "game-state-request" {
        return json!({"ok":true,"state":public(&room.data,profiles)});
    }
    if event == "game-restart-request" {
        if !room.data["restartVotes"].is_object() {
            room.data["restartVotes"] = json!({});
        }
        room.data["restartVotes"][key(actor)] = json!(true);
        if !room.data["players"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["isBot"] != true)
            .all(|p| room.data["restartVotes"][key(p["name"].as_str().unwrap())] == true)
        {
            events.push(Event {
                names: names(&room.data)
                    .into_iter()
                    .filter(|n| key(n) != key(actor) && player(&room.data, n).is_some())
                    .collect(),
                event: "game-restart-consent",
                payload: json!({"id":room.data["id"],"from":actor}),
            });
            return json!({"ok":true,"pending":true});
        }
        stop(room);
        room.data["restartVotes"] = json!({});
        room.data["countdown"] = json!(0);
        room.data["soloCountdown"] = json!(0);
        room.data["turn"] = json!(0);
        room.data["winner"] = Value::Null;
        room.data["draw"] = json!(false);
        room.data["dice"] = json!(0);
        room.data["lastAction"] = json!({"id":id(),"type":"restart","ts":time});
        if room.data["type"] == "ludo" {
            colors(&mut room.data);
            room.data["ludo"] =
                json!({"pawns":[[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1]]});
            room.data["status"] = json!("lobby");
            bump(room, events, profiles, time);
            countdown(room, events, profiles, time);
            solo(room, events, profiles, time);
        } else {
            start(&mut room.data, false);
            bump(room, events, profiles, time);
            schedule(room, events, profiles, time);
        }
        return json!({"ok":true,"restarted":true,"state":public(&room.data,profiles)});
    }
    if room.data["status"] != "playing" {
        return fail("Game tidak aktif");
    }
    let action_type = if event == "ludo-move" { "move" } else { "roll" };
    let client_id = input["clientActionId"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect::<String>();
    let recent_key = format!("{}|{action_type}|{client_id}", key(actor));
    if event != "chess-move" && !client_id.is_empty() {
        if let Some((_, previous)) = room.recent.iter().find(|(k, _)| k == &recent_key) {
            let mut reply = previous.clone();
            reply["duplicate"] = json!(true);
            reply["state"] = public(&room.data, profiles);
            return reply;
        }
    }
    if room.data["turn"] != pi {
        return fail("Bukan giliran kamu");
    }
    let reply = match event {
        "chess-move" if room.data["type"] == "chess" => {
            let Some(from) = input["from"].as_u64().filter(|v| *v < 64) else {
                return fail("Langkah tidak valid atau membuat raja terkena skak");
            };
            let Some(to) = input["to"].as_u64().filter(|v| *v < 64) else {
                return fail("Langkah tidak valid atau membuat raja terkena skak");
            };
            let b = chess::board(&room.data["board"]).unwrap();
            if !chess::legal(
                &b,
                from as usize,
                to as usize,
                pi as i32,
                &room.data["castlingRights"],
                room.data["enPassantTarget"].as_i64().unwrap_or(-1),
            ) {
                return fail(if chess::in_check(&b, pi as i32) {
                    "Raja sedang SKAK. Langkah harus menghilangkan skak."
                } else {
                    "Langkah tidak valid atau membuat raja terkena skak"
                });
            }
            reset_idle(&mut room.data, pi);
            chess::apply(&mut room.data, from as usize, to as usize, pi, &id(), time);
            bump(room, events, profiles, time);
            schedule(room, events, profiles, time);
            json!({"ok":true})
        }
        "snakes-roll" if room.data["type"] == "snakes" => {
            reset_idle(&mut room.data, pi);
            snakes_roll(room, pi, 1 + random(6) as u64, events, profiles, time)
        }
        "ludo-roll" if room.data["type"] == "ludo" => {
            if room.data["dice"].as_u64().unwrap_or_default() > 0 {
                return fail("Pilih pion dulu");
            }
            let d = 1 + random(6) as u64;
            room.data["dice"] = json!(d);
            let legal = ludo_legal(&room.data, pi, d);
            let power = input["power"]
                .as_f64()
                .filter(|v| v.is_finite() && *v != 0.0)
                .unwrap_or(50.0)
                .clamp(0.0, 100.0);
            room.data["lastAction"] = json!({"id":id(),"type":"ludo-roll","player":pi,"dice":d,"power":power,"ts":time,"clientActionId":client_id});
            bump(room, events, profiles, time);
            if legal.is_empty() {
                reset_idle(&mut room.data, pi);
                room.automatic = Some((time + 850, pi, d, false));
            }
            json!({"ok":true,"dice":d,"legal":legal,"revision":room.data["revision"]})
        }
        "ludo-move" if room.data["type"] == "ludo" => {
            let Some(pawn) = input["pawn"].as_u64().filter(|v| *v < 4) else {
                return fail("Langkah tidak valid");
            };
            let d = room.data["dice"].as_u64().unwrap_or_default();
            if d == 0 {
                return fail("Langkah tidak valid");
            }
            let pos = room.data["ludo"]["pawns"][pi][pawn as usize]
                .as_i64()
                .unwrap();
            if pos < 0 && d != 6 {
                return fail("Butuh angka 6 untuk keluar rumah");
            }
            if pos >= 0 && pos + d as i64 > 57 {
                return fail("Harus pas untuk masuk finish");
            }
            room.automatic = None;
            reset_idle(&mut room.data, pi);
            ludo_move(room, pi, pawn as usize, d, events, profiles, time);
            json!({"ok":true,"revision":room.data["revision"]})
        }
        _ => return fail("Aksi tidak cocok dengan jenis permainan"),
    };
    if event != "chess-move" && !client_id.is_empty() {
        let mut saved = reply.clone();
        saved["clientActionId"] = json!(client_id);
        room.recent.push_back((recent_key, saved));
        if room.recent.len() > 80 {
            room.recent.pop_front();
        }
    }
    reply
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn automatic_turns_and_idle_cancellation() {
        let directory =
            std::env::temp_dir().join(format!("kc-game-timer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let profiles = Arc::new(Profiles::open(directory.clone()).unwrap());
        let server = Server::open(directory.join("games.json"), profiles).unwrap();
        assert!(server.timer_delay().is_none());
        let time = now();
        let make = |name: &str, kind: &str, first_is_bot| {
            let mut game = json!({"id":name,"type":kind,"host":"Alice","invitedNames":[],"declinedNames":[],
                "players":[{"name":if first_is_bot {"Bot Ceria"}else{"Alice"},"isBot":first_is_bot,"color":"red"},
                    {"name":"Bob","isBot":false,"color":"green"}]});
            if kind == "ludo" {
                game["ludo"] =
                    json!({"pawns":[[20,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1],[-1,-1,-1,-1]]});
            }
            start(&mut game, false);
            let mut room = Room::new(game);
            schedule(&mut room, &mut Vec::new(), &server.profiles, time);
            room
        };
        let ludo = make("ludo-bot", "ludo", true);
        let snakes = make("snakes-bot", "snakes", true);
        let mut idle = make("idle", "chess", false);
        idle.data["idleMisses"]["alice"] = json!(1);
        server.rooms.lock().unwrap().extend([ludo, snakes, idle]);
        assert!(server.tick_at(time + 699).unwrap().is_empty());
        let updates = server.tick_at(time + 700).unwrap();
        let ludo_roll = updates
            .iter()
            .find(|event| event.payload["id"] == "ludo-bot")
            .unwrap();
        assert_eq!(ludo_roll.payload["lastAction"]["type"], "ludo-roll");
        let roll = updates
            .iter()
            .find(|event| event.payload["id"] == "snakes-bot")
            .unwrap();
        let dice = roll.payload["lastAction"]["dice"].as_u64().unwrap();
        let expected = snakes_board()["ladders"][dice.to_string()]
            .as_u64()
            .unwrap_or(dice);
        assert_eq!(roll.payload["snakesLadders"]["positions"][0], expected);
        assert_eq!(roll.payload["turn"], 1);
        let updates = server.tick_at(time + 1350).unwrap();
        let move_event = updates
            .iter()
            .find(|event| event.payload["id"] == "ludo-bot")
            .unwrap();
        assert_eq!(move_event.payload["lastAction"]["type"], "ludo-move");
        assert_eq!(move_event.payload["dice"], 0);
        assert!(server
            .tick_at(time + 30000)
            .unwrap()
            .iter()
            .any(|event| event.payload["id"] == "idle"));
        {
            let rooms = server.rooms.lock().unwrap();
            let idle = rooms.iter().find(|room| room.data["id"] == "idle").unwrap();
            assert_eq!(idle.data["idleMisses"]["alice"], 2);
            assert_eq!(idle.data["turn"], 1);
        }
        assert!(server
            .tick_at(time + 60000)
            .unwrap()
            .iter()
            .any(|event| event.payload["id"] == "idle"));
        let updates = server.tick_at(time + 90000).unwrap();
        assert!(updates.iter().any(|event| event.event == "game-closed"
            && event.payload["id"] == "idle"
            && event.payload["reason"] == "idle-3x"
            && event.payload["by"] == "Alice"));
        let saved: Vec<Value> =
            serde_json::from_slice(&std::fs::read(directory.join("games.json")).unwrap()).unwrap();
        assert!(!saved.iter().any(|room| room["id"] == "idle"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
