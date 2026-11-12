//! Accounts and server-provided resources.
//!
//! Accounts live in memory and are written to `<data-dir>/accounts/<XPID>.json`
//! off the hot path. Resources (config documents, EULA, channel info, login
//! settings) are embedded defaults that a file of the same name in
//! `<data-dir>/resources/` overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
}

pub fn hash_secret(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

impl Store {
    pub fn open(dir: &Path, unlock_all: bool) -> Result<Arc<Self>> {
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
        }))
    }

    pub fn account_count(&self) -> usize {
        self.accounts.lock().len()
    }

    /// Raw resource bytes (JSON), override file first.
    pub fn resource(&self, name: &str) -> Option<Arc<Vec<u8>>> {
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
    /// to a password, as EchoRelay's `?auth=` does.
    pub fn login(&self, xpid: XpId, display_name: &str, auth: Option<&str>) -> Result<Account, String> {
        let mut accounts = self.accounts.lock();
        let a = accounts.entry(xpid).or_insert_with(|| Account {
            xpid: xpid.token(),
            display_name: String::new(),
            auth_hash: None,
            client_profile: serde_json::from_str(DEFAULT_CLIENT_PROFILE).unwrap_or(json!({})),
            server_profile: serde_json::from_str(DEFAULT_SERVER_PROFILE).unwrap_or(json!({})),
            banned_until: None,
        });
        if let Some(until) = a.banned_until {
            let now = now_secs();
            if until > now {
                return Err(format!("Account banned for {} more minutes", (until - now) / 60 + 1));
            }
        }
        match (&a.auth_hash, auth.filter(|s| !s.is_empty())) {
            (Some(h), Some(p)) if *h != hash_secret(p) => return Err("Incorrect account password".into()),
            (Some(_), None) => return Err("This account is password protected (add auth=... to loginservice_host)".into()),
            (None, Some(p)) => a.auth_hash = Some(hash_secret(p)),
            _ => {}
        }
        let name = sanitize_name(display_name);
        if !name.is_empty() {
            a.display_name = name;
        } else if a.display_name.is_empty() {
            a.display_name = format!("Player{}", xpid.account % 100_000);
        }
        let out = a.clone();
        self.save(&out);
        Ok(out)
    }

    /// `{"client": ..., "server": ...}` for the logged-in user.
    pub fn profiles_json(&self, xpid: XpId, login: &Value) -> Option<Vec<u8>> {
        let accounts = self.accounts.lock();
        let a = accounts.get(&xpid)?;
        let mut client = a.client_profile.clone();
        client["displayname"] = json!(a.display_name);
        client["xplatformid"] = json!(a.xpid);
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
        s["displayname"] = json!(a.display_name);
        s["xplatformid"] = json!(a.xpid);
        if let Some(lock) = login.get("publisher_lock").and_then(Value::as_str) {
            s["publisher_lock"] = json!(lock);
        }
        if let Some(v) = login.get("lobbyversion").and_then(Value::as_u64) {
            s["lobbyversion"] = json!(v);
        }
        if self.unlock_all {
            let arena: serde_json::Map<String, Value> =
                UNLOCKS_ARENA.lines().filter(|l| !l.is_empty()).map(|l| (l.to_owned(), json!(true))).collect();
            let combat: serde_json::Map<String, Value> =
                UNLOCKS_COMBAT.lines().filter(|l| !l.is_empty()).map(|l| (l.to_owned(), json!(true))).collect();
            s["unlocks"] = json!({"arena": arena, "combat": combat});
        }
        s
    }

    pub fn update_client_profile(&self, xpid: XpId, mut profile: Value) {
        let mut accounts = self.accounts.lock();
        if let Some(a) = accounts.get_mut(&xpid) {
            // The server owns identity fields.
            if let Some(o) = profile.as_object_mut() {
                o.remove("displayname");
                o.remove("xplatformid");
            }
            profile["modifytime"] = json!(now_secs());
            a.client_profile = profile;
            let out = a.clone();
            drop(accounts);
            self.save(&out);
        }
    }

    /// A loadout reported by a game server: slot name → item symbol.
    pub fn save_loadout(&self, xpid: XpId, slots: &[(&str, u64)]) {
        let mut accounts = self.accounts.lock();
        let Some(a) = accounts.get_mut(&xpid) else { return };
        let target = &mut a.server_profile["loadout"]["instances"]["unified"]["slots"];
        if !target.is_object() {
            *target = json!({});
        }
        let mut changed = false;
        for (slot, hash) in slots {
            if let Some(name) = self.cosmetic_names.get(hash) {
                target[*slot] = json!(name);
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
