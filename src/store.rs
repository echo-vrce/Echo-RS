//! Accounts and server-provided resources.
//!
//! Accounts live in memory and are written to `<data-dir>/accounts/<XPID>.json`
//! off the hot path. Resources (config documents, EULA, channel info, login
//! settings) are embedded defaults that a file of the same name in
//! `<data-dir>/resources/` overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::sns::{XpId, symbol};

const DEFAULT_CLIENT_PROFILE: &str = include_str!("../data/default_client_profile.json");
const DEFAULT_SERVER_PROFILE: &str = include_str!("../data/default_server_profile.json");
const UNLOCKS_ARENA: &str = include_str!("../data/unlocks_arena.txt");
const UNLOCKS_COMBAT: &str = include_str!("../data/unlocks_combat.txt");

fn embedded_resource(name: &str) -> Option<&'static str> {
    Some(match name {
        "game_settings" => include_str!("../data/game_settings.json"),
        "channel_info" => include_str!("../data/channel_info.json"),
        "eula" => include_str!("../data/eula.json"),
        "config_main_menu" => include_str!("../data/config_main_menu.json"),
        "config_active_battle_pass_season" => include_str!("../data/config_active_battle_pass_season.json"),
        "config_active_store_entry" => include_str!("../data/config_active_store_entry.json"),
        "config_active_store_featured_entry" => include_str!("../data/config_active_store_featured_entry.json"),
        _ => return None,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub xpid: String,
    pub display_name: String,
    /// SHA-256 of the `auth` login parameter, locked on first login.
    #[serde(default)]
    pub auth_hash: Option<String>,
    pub client_profile: Value,
    pub server_profile: Value,
    #[serde(default)]
    pub banned_until: Option<u64>,
}

pub struct Store {
    dir: PathBuf,
    accounts: Mutex<HashMap<XpId, Account>>,
    resources: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    unlock_all: bool,
    /// Cosmetic symbol hash → item name, for loadouts reported by game servers.
    cosmetic_names: HashMap<u64, &'static str>,
    write_tx: tokio::sync::mpsc::UnboundedSender<(PathBuf, Vec<u8>)>,
    /// Refuse logins without a real password (`--require-password`).
    require_password: bool,
    /// Wrong passwords per account.
    failures: Mutex<HashMap<XpId, Failures>>,
}

/// An account's wrong passwords: (count, first failure, locked until).
type Failures = (u32, Instant, Option<Instant>);

/// The `auth` value of the generated client config: not a real password.
pub const PLACEHOLDER_AUTH: &str = "CHANGE_ME";
/// Wrong passwords allowed before an account is locked for a while.
const MAX_FAILURES: u32 = 5;
/// The window the failures are counted in.
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// First lockout; each further failure doubles it, up to `MAX_LOCKOUT`.
const LOCKOUT: Duration = Duration::from_secs(60);
const MAX_LOCKOUT: Duration = Duration::from_secs(60 * 60);
const PBKDF2_ROUNDS: u32 = 100_000;

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// The old account hash: unsalted SHA-256 (still accepted, then upgraded).
fn legacy_hash(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// PBKDF2-HMAC-SHA256, one 32-byte block.
fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mac = <Hmac<Sha256> as Mac>::new_from_slice(password).expect("HMAC takes any key length");
    let mut m = mac.clone();
    m.update(salt);
    m.update(&1u32.to_be_bytes());
    let mut u: [u8; 32] = m.finalize().into_bytes().into();
    let mut out = u;
    for _ in 1..rounds {
        let mut m = mac.clone();
        m.update(&u);
        u = m.finalize().into_bytes().into();
        out.iter_mut().zip(u).for_each(|(o, x)| *o ^= x);
    }
    out
}

/// A salted, slow hash of a password: `pbkdf2$<rounds>$<salt>$<hash>`.
pub fn hash_secret(s: &str) -> String {
    let salt: [u8; 16] = rand::random();
    format!("pbkdf2${PBKDF2_ROUNDS}${}${}", hex(&salt), hex(&pbkdf2(s.as_bytes(), &salt, PBKDF2_ROUNDS)))
}

use crate::wire::crypto::ct_eq;

/// Whether `password` matches a stored hash (either form).
fn verify_secret(stored: &str, password: &str) -> bool {
    let mut parts = stored.split('$');
    match (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("pbkdf2"), Some(rounds), Some(salt), Some(hash), None) => {
            let (Ok(rounds), Some(salt), Some(hash)) = (rounds.parse::<u32>(), unhex(salt), unhex(hash)) else { return false };
            (1..=10_000_000).contains(&rounds) && ct_eq(&pbkdf2(password.as_bytes(), &salt, rounds), &hash)
        }
        _ => ct_eq(stored.as_bytes(), legacy_hash(password).as_bytes()),
    }
}

/// A resource name a client may ask for: letters, digits, `_` and `-` only,
/// so it can never leave `<data-dir>/resources/`.
fn valid_resource_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `v` as an object (replacing anything else), for writing keys into it.
/// `value["key"] = ...` panics on anything but an object or null, and a
/// panic aborts the whole server.
fn as_object(v: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !v.is_object() {
        *v = json!({});
    }
    v.as_object_mut().expect("just made an object")
}

/// The object at `path` under `v`, made (or replaced) where needed.
fn object_at<'a>(mut v: &'a mut Value, path: &[&str]) -> &'a mut serde_json::Map<String, Value> {
    for k in path {
        v = as_object(v).entry((*k).to_owned()).or_insert(Value::Null);
    }
    as_object(v)
}

/// Limits on what game servers' stats may add to one account.
const MAX_STAT_GROUPS: usize = 64;
const MAX_STATS_PER_GROUP: usize = 512;

impl Store {
    pub fn open(dir: &Path, unlock_all: bool, require_password: bool) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir.join("accounts"))?;
        std::fs::create_dir_all(dir.join("resources"))?;
        let mut accounts = HashMap::new();
        for entry in std::fs::read_dir(dir.join("accounts"))? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Account>(&b)?)) {
                    Ok(a) => {
                        if let Some(id) = XpId::parse(&a.xpid) {
                            accounts.insert(id, a);
                        }
                    }
                    Err(e) => warn!(path = %path.display(), error = %e, "skipping unreadable account file"),
                }
            }
        }
        // One background writer: account saves never block a request.
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<(PathBuf, Vec<u8>)>();
        tokio::spawn(async move {
            while let Some((path, data)) = write_rx.recv().await {
                let tmp = path.with_extension("tmp");
                let r = tokio::fs::write(&tmp, &data).await;
                if let Err(e) = match r {
                    Ok(()) => tokio::fs::rename(&tmp, &path).await,
                    Err(e) => Err(e),
                } {
                    warn!(path = %path.display(), error = %e, "account save failed");
                }
            }
        });
        let cosmetic_names = UNLOCKS_ARENA
            .lines()
            .chain(UNLOCKS_COMBAT.lines())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| (symbol(s), s))
            .collect();
        Ok(Arc::new(Self {
            dir: dir.to_owned(),
            accounts: Mutex::new(accounts),
            resources: Mutex::new(HashMap::new()),
            unlock_all,
            cosmetic_names,
            write_tx,
            require_password,
            failures: Mutex::new(HashMap::new()),
        }))
    }

    pub fn account_count(&self) -> usize {
        self.accounts.lock().len()
    }

    pub fn exists(&self, xpid: XpId) -> bool {
        self.accounts.lock().contains_key(&xpid)
    }

    /// Raw resource bytes (JSON), override file first. Only plain names
    /// (`valid_resource_name`): clients choose them.
    pub fn resource(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        if !valid_resource_name(name) {
            return None;
        }
        if let Some(r) = self.resources.lock().get(name) {
            return Some(r.clone());
        }
        let path = self.dir.join("resources").join(format!("{name}.json"));
        let data = match std::fs::read(&path) {
            Ok(b) => {
                // Re-serialize compactly; a broken override falls back to the default.
                match serde_json::from_slice::<Value>(&b) {
                    Ok(v) => serde_json::to_vec(&v).ok()?,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "invalid resource override; using default");
                        compact(embedded_resource(name)?)
                    }
                }
            }
            Err(_) => compact(embedded_resource(name)?),
        };
        let data = Arc::new(data);
        self.resources.lock().insert(name.to_owned(), data.clone());
        Some(data)
    }

    fn save(&self, a: &Account) {
        let path = self.dir.join("accounts").join(format!("{}.json", a.xpid));
        if let Ok(data) = serde_json::to_vec_pretty(a) {
            drop(self.write_tx.send((path, data)));
        }
    }

    /// Log in (creating the account on first sight). `auth` locks the account
    /// to a password, as EchoRelay's `?auth=` does. The (deliberately slow)
    /// password check runs outside the accounts lock, and repeated wrong
    /// passwords lock the account for a while.
    pub fn login(&self, xpid: XpId, display_name: &str, auth: Option<&str>) -> Result<Account, String> {
        let auth = auth.filter(|s| !s.is_empty());
        if self.require_password && auth.is_none_or(|p| p == PLACEHOLDER_AUTH) {
            return Err("This server needs a password: set auth=... in loginservice_host (not CHANGE_ME)".into());
        }
        let stored = {
            let accounts = self.accounts.lock();
            match accounts.get(&xpid) {
                Some(a) => {
                    if let Some(until) = a.banned_until {
                        let now = now_secs();
                        if until > now {
                            return Err(format!("Account banned for {} more minutes", (until - now) / 60 + 1));
                        }
                    }
                    a.auth_hash.clone()
                }
                None => None,
            }
        };
        let mut new_hash = None;
        match (&stored, auth) {
            (Some(h), Some(p)) => {
                if let Some(left) = self.locked_for(xpid) {
                    return Err(format!("Too many wrong passwords; try again in {} s", left.as_secs() + 1));
                }
                if !verify_secret(h, p) {
                    self.note_failure(xpid);
                    return Err("Incorrect account password".into());
                }
                self.failures.lock().remove(&xpid);
                if !h.starts_with("pbkdf2$") {
                    new_hash = Some(hash_secret(p));
                }
            }
            (Some(_), None) => return Err("This account is password protected (add auth=... to loginservice_host)".into()),
            (None, Some(p)) => new_hash = Some(hash_secret(p)),
            (None, None) => {}
        }

        let mut accounts = self.accounts.lock();
        let a = accounts.entry(xpid).or_insert_with(|| Account {
            xpid: xpid.token(),
            display_name: String::new(),
            auth_hash: None,
            client_profile: serde_json::from_str(DEFAULT_CLIENT_PROFILE).unwrap_or(json!({})),
            server_profile: serde_json::from_str(DEFAULT_SERVER_PROFILE).unwrap_or(json!({})),
            banned_until: None,
        });
        // Another login set or changed the password while this one checked.
        if a.auth_hash != stored {
            return Err("Another login for this account got there first; try again".into());
        }
        if let Some(h) = new_hash {
            a.auth_hash = Some(h);
        }
        let name = sanitize_name(display_name);
        if !name.is_empty() {
            a.display_name = name;
        } else if a.display_name.is_empty() {
            a.display_name = format!("Player{}", xpid.account % 100_000);
        }
        let out = a.clone();
        drop(accounts);
        self.save(&out);
        Ok(out)
    }

    /// How much longer this account refuses passwords, if it does.
    fn locked_for(&self, xpid: XpId) -> Option<Duration> {
        let until = self.failures.lock().get(&xpid)?.2?;
        until.checked_duration_since(Instant::now())
    }

    fn note_failure(&self, xpid: XpId) {
        let now = Instant::now();
        let mut f = self.failures.lock();
        if f.len() > 100_000 {
            f.retain(|_, (_, first, until)| now.duration_since(*first) < FAILURE_WINDOW || until.is_some_and(|u| u > now));
        }
        let e = f.entry(xpid).or_insert((0, now, None));
        if now.duration_since(e.1) > FAILURE_WINDOW && e.2.is_none_or(|u| u <= now) {
            *e = (0, now, None);
        }
        e.0 += 1;
        if e.0 >= MAX_FAILURES {
            let lock = (LOCKOUT * 2u32.pow((e.0 - MAX_FAILURES).min(6))).min(MAX_LOCKOUT);
            e.2 = Some(now + lock);
            warn!(player = %xpid, failures = e.0, secs = lock.as_secs(), "wrong passwords: account locked for a while");
        }
    }

    /// `{"client": ..., "server": ...}` for the logged-in user.
    pub fn profiles_json(&self, xpid: XpId, login: &Value) -> Option<Vec<u8>> {
        let accounts = self.accounts.lock();
        let a = accounts.get(&xpid)?;
        let mut client = a.client_profile.clone();
        let c = as_object(&mut client);
        c.insert("displayname".into(), json!(a.display_name));
        c.insert("xplatformid".into(), json!(a.xpid));
        let server = self.server_profile_value(a, login);
        serde_json::to_vec(&json!({"client": client, "server": server})).ok()
    }

    pub fn server_profile_json(&self, xpid: XpId) -> Option<Vec<u8>> {
        let accounts = self.accounts.lock();
        let a = accounts.get(&xpid)?;
        serde_json::to_vec(&self.server_profile_value(a, &Value::Null)).ok()
    }

    fn server_profile_value(&self, a: &Account, login: &Value) -> Value {
        let mut s = a.server_profile.clone();
        let o = as_object(&mut s);
        o.insert("displayname".into(), json!(a.display_name));
        o.insert("xplatformid".into(), json!(a.xpid));
        if let Some(lock) = login.get("publisher_lock").and_then(Value::as_str) {
            o.insert("publisher_lock".into(), json!(lock));
        }
        if let Some(v) = login.get("lobbyversion").and_then(Value::as_u64) {
            o.insert("lobbyversion".into(), json!(v));
        }
        if self.unlock_all {
            let arena: serde_json::Map<String, Value> =
                UNLOCKS_ARENA.lines().filter(|l| !l.is_empty()).map(|l| (l.to_owned(), json!(true))).collect();
            let combat: serde_json::Map<String, Value> =
                UNLOCKS_COMBAT.lines().filter(|l| !l.is_empty()).map(|l| (l.to_owned(), json!(true))).collect();
            o.insert("unlocks".into(), json!({"arena": arena, "combat": combat}));
        }
        s
    }

    /// A client's own profile. Anything but a JSON object is refused
    /// (`false`).
    pub fn update_client_profile(&self, xpid: XpId, mut profile: Value) -> bool {
        let Some(o) = profile.as_object_mut() else { return false };
        // The server owns identity fields.
        o.remove("displayname");
        o.remove("xplatformid");
        o.insert("modifytime".into(), json!(now_secs()));
        let mut accounts = self.accounts.lock();
        if let Some(a) = accounts.get_mut(&xpid) {
            a.client_profile = profile;
            let out = a.clone();
            drop(accounts);
            self.save(&out);
        }
        true
    }

    /// A game server's stat changes for a player (`{"stats": {group: {name:
    /// {"op", "val"}}}}`, as native's `SNSRefreshProfileFromServer` carries
    /// them): kept in the server profile the way it stores stats,
    /// `{"cnt", "op", "val"}`; `add` sums, `max` keeps the larger, `rep`
    /// replaces.
    pub fn apply_stats(&self, xpid: XpId, update: &Value) {
        let Some(groups) = update.get("stats").and_then(Value::as_object) else { return };
        let mut accounts = self.accounts.lock();
        let Some(a) = accounts.get_mut(&xpid) else { return };
        let all = object_at(&mut a.server_profile, &["stats"]);
        for (group, stats) in groups {
            let Some(stats) = stats.as_object() else { continue };
            if !all.contains_key(group) && all.len() >= MAX_STAT_GROUPS {
                continue;
            }
            let g = as_object(all.entry(group.clone()).or_insert(Value::Null));
            for (name, change) in stats {
                if !g.contains_key(name) && g.len() >= MAX_STATS_PER_GROUP {
                    continue;
                }
                let op = change.get("op").and_then(Value::as_str).unwrap_or("add");
                let Some(v) = change.get("val").and_then(Value::as_f64) else { continue };
                let cur = g.entry(name.clone()).or_insert(Value::Null);
                let old = cur.get("val").and_then(Value::as_f64);
                let cnt = cur.get("cnt").and_then(Value::as_u64).unwrap_or(0) + 1;
                let new = match (op, old) {
                    ("add", Some(o)) => o + v,
                    ("max", Some(o)) => o.max(v),
                    _ => v,
                };
                let whole = change.get("val").is_some_and(|x| x.is_u64() || x.is_i64()) && old.is_none_or(|o| o.fract() == 0.0);
                let val = if whole { json!(new as i64) } else { json!(new) };
                *cur = json!({"cnt": cnt, "op": op, "val": val});
            }
        }
        let out = a.clone();
        drop(accounts);
        self.save(&out);
    }

    /// A loadout reported by a game server: slot name → item symbol.
    pub fn save_loadout(&self, xpid: XpId, slots: &[(&str, u64)]) {
        let mut accounts = self.accounts.lock();
        let Some(a) = accounts.get_mut(&xpid) else { return };
        let target = object_at(&mut a.server_profile, &["loadout", "instances", "unified", "slots"]);
        let mut changed = false;
        for (slot, hash) in slots {
            if let Some(name) = self.cosmetic_names.get(hash) {
                target.insert((*slot).to_owned(), json!(name));
                changed = true;
            } else if *hash != 0 {
                debug!(slot, hash = format_args!("{hash:#018x}"), "unknown cosmetic symbol in loadout");
            }
        }
        if changed {
            let out = a.clone();
            drop(accounts);
            self.save(&out);
        }
    }

    pub fn ban(&self, xpid: XpId, minutes: u64) -> bool {
        let mut accounts = self.accounts.lock();
        let Some(a) = accounts.get_mut(&xpid) else { return false };
        a.banned_until = (minutes > 0).then(|| now_secs() + minutes * 60);
        let out = a.clone();
        drop(accounts);
        self.save(&out);
        true
    }
}

fn compact(s: &str) -> Vec<u8> {
    serde_json::from_str::<Value>(s).ok().and_then(|v| serde_json::to_vec(&v).ok()).unwrap_or_else(|| s.as_bytes().to_vec())
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn sanitize_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(20).collect::<String>().trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(require_password: bool) -> (Arc<Store>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ers-store-test-{}-{}", std::process::id(), rand::random::<u32>()));
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let _g = rt.enter();
        let s = Store::open(&dir, true, require_password).expect("open");
        std::mem::forget(rt);
        (s, dir)
    }

    const X: XpId = XpId { platform: 4, account: 77 };

    #[test]
    fn resource_names_cannot_leave_the_resources_folder() {
        for bad in ["../accounts/OVR_ORG-1", "..", "a/b", r"a\b", "C:x", "/etc/passwd", "", "eula.json"] {
            assert!(!valid_resource_name(bad), "{bad:?}");
        }
        for good in ["eula", "config_main_menu", "channel_info", "a-b"] {
            assert!(valid_resource_name(good), "{good:?}");
        }
        let (s, dir) = store(false);
        assert!(s.resource("../accounts/x").is_none());
        assert!(s.resource("eula").is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_profile_that_is_not_an_object_is_refused_not_a_panic() {
        let (s, dir) = store(false);
        s.login(X, "a", None).expect("login");
        for v in [json!([1]), json!(5), json!("x"), Value::Null] {
            assert!(!s.update_client_profile(X, v));
        }
        assert!(s.update_client_profile(X, json!({"a": 1})));
        assert!(s.profiles_json(X, &Value::Null).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stats_and_loadouts_survive_odd_profiles() {
        let (s, dir) = store(false);
        s.login(X, "a", None).expect("login");
        s.accounts.lock().get_mut(&X).expect("acct").server_profile = json!({"stats": [1], "loadout": 3});
        s.apply_stats(X, &json!({"stats": {"g": {"n": {"op": "add", "val": 2}}}}));
        let first = s.cosmetic_names.iter().next().map(|(h, _)| *h).expect("a cosmetic");
        s.save_loadout(X, &[("banner", first)]);
        let a = s.accounts.lock().get(&X).cloned().expect("acct");
        assert_eq!(a.server_profile["stats"]["g"]["n"]["val"], json!(2));
        assert!(a.server_profile["loadout"]["instances"]["unified"]["slots"]["banner"].is_string());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn passwords_are_salted_and_old_hashes_still_work() {
        let a = hash_secret("pw");
        let b = hash_secret("pw");
        assert_ne!(a, b, "salted");
        assert!(verify_secret(&a, "pw") && !verify_secret(&a, "nope"));
        assert!(verify_secret(&legacy_hash("pw"), "pw") && !verify_secret(&legacy_hash("pw"), "x"));
        assert!(!verify_secret("pbkdf2$0$00$00", "pw"));
    }

    #[test]
    fn wrong_passwords_lock_the_account() {
        let (s, dir) = store(false);
        s.login(X, "a", Some("right")).expect("first login sets it");
        // A legacy hash is upgraded on the next good login.
        s.accounts.lock().get_mut(&X).expect("acct").auth_hash = Some(legacy_hash("right"));
        s.login(X, "a", Some("right")).expect("legacy ok");
        assert!(s.accounts.lock().get(&X).and_then(|a| a.auth_hash.clone()).is_some_and(|h| h.starts_with("pbkdf2$")));
        for _ in 0..MAX_FAILURES {
            assert!(s.login(X, "a", Some("wrong")).is_err());
        }
        let e = s.login(X, "a", Some("right")).expect_err("locked");
        assert!(e.contains("Too many"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn require_password_refuses_none_and_the_placeholder() {
        let (s, dir) = store(true);
        assert!(s.login(X, "a", None).is_err());
        assert!(s.login(X, "a", Some(PLACEHOLDER_AUTH)).is_err());
        assert!(s.login(X, "a", Some("real")).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
