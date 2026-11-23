//! The protobuf messages of the game-server fleet socket (package
//! `rtapi.v1`): what a game server and its backend (our lobby, or nakama
//! through the upstream bridge) send each other.
//!
//! Only the messages this project uses are here. The tags and field types are
//! the wire contract with nakama: never renumber them. An envelope carrying a
//! message not listed here decodes with `message: None`.

use std::collections::HashMap;

#[derive(Clone, PartialEq, prost::Message)]
pub struct Envelope {
    #[prost(string, tag = "1")]
    pub cid: String,
    #[prost(oneof = "envelope::Message", tags = "2, 5, 6, 7, 8, 9, 10, 11, 12, 35, 47, 51, 53")]
    pub message: Option<envelope::Message>,
}

pub mod envelope {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Message {
        #[prost(message, tag = "2")]
        Error(super::Error),
        #[prost(message, tag = "5")]
        GameServerRegistration(super::GameServerRegistrationMessage),
        #[prost(message, tag = "6")]
        GameServerRegistrationSuccess(super::GameServerRegistrationSuccessMessage),
        #[prost(message, tag = "7")]
        LobbySessionCreate(super::LobbySessionCreateMessage),
        #[prost(message, tag = "8")]
        LobbySessionEvent(super::LobbySessionEventMessage),
        #[prost(message, tag = "9")]
        LobbyEntrantConnected(super::LobbyEntrantsConnectedMessage),
        #[prost(message, tag = "10")]
        LobbyEntrantsAccept(super::LobbyEntrantsAcceptMessage),
        #[prost(message, tag = "11")]
        LobbyEntrantReject(super::LobbyEntrantsRejectMessage),
        #[prost(message, tag = "12")]
        LobbyEntrantRemoved(super::LobbyEntrantRemovedMessage),
        #[prost(message, tag = "35")]
        LobbySessionSuccessV5(super::SnsLobbySessionSuccessV5Message),
        #[prost(message, tag = "47")]
        RemoteLogSetV3(super::SnsRemoteLogSetV3Message),
        #[prost(message, tag = "51")]
        UserServerProfileUpdateRequest(super::SnsUserServerProfileUpdateRequestMessage),
        #[prost(message, tag = "53")]
        GameServerSaveLoadout(super::GameServerSaveLoadoutMessage),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Error {
    #[prost(int32, tag = "1")]
    pub code: i32,
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(map = "string, string", tag = "3")]
    pub context: HashMap<String, String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbySessionEventMessage {
    #[prost(string, tag = "1")]
    pub lobby_session_id: String,
    #[prost(int32, tag = "2")]
    pub code: i32,
}

pub mod lobby_session_event_message {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
    #[repr(i32)]
    pub enum Code {
        Unspecified = 0,
        Started = 1,
        Ended = 2,
        Locked = 3,
        Unlocked = 4,
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GameServerRegistrationMessage {
    #[prost(string, tag = "1")]
    pub login_session_id: String,
    #[prost(uint64, tag = "2")]
    pub server_id: u64,
    #[prost(string, tag = "3")]
    pub internal_ip_address: String,
    #[prost(uint32, tag = "4")]
    pub port: u32,
    #[prost(uint64, tag = "5")]
    pub region: u64,
    #[prost(uint64, tag = "6")]
    pub version_lock: u64,
    #[prost(uint32, tag = "7")]
    pub time_step_usecs: u32,
    #[prost(string, tag = "8")]
    pub version: String,
    /// The levels this server can load (symbols). Atlas only gives a server
    /// matches on a level in this list; empty means every level.
    #[prost(fixed64, repeated, tag = "9")]
    pub installed_levels: Vec<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GameServerRegistrationSuccessMessage {
    #[prost(uint64, tag = "1")]
    pub server_id: u64,
    #[prost(string, tag = "2")]
    pub external_ip_address: String,
}

/// One cosmetic: slot type and item, both symbol hashes.
#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadoutItem {
    #[prost(fixed64, tag = "1")]
    pub slot_type: u64,
    #[prost(fixed64, tag = "2")]
    pub equipped_item: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadoutSlot {
    #[prost(fixed64, tag = "1")]
    pub selectionmode: u64,
    #[prost(fixed64, tag = "2")]
    pub banner: u64,
    #[prost(fixed64, tag = "3")]
    pub booster: u64,
    #[prost(fixed64, tag = "4")]
    pub bracer: u64,
    #[prost(fixed64, tag = "5")]
    pub chassis: u64,
    #[prost(fixed64, tag = "6")]
    pub decal: u64,
    #[prost(fixed64, tag = "7")]
    pub decal_body: u64,
    #[prost(fixed64, tag = "8")]
    pub emissive: u64,
    #[prost(fixed64, tag = "9")]
    pub emote: u64,
    #[prost(fixed64, tag = "10")]
    pub secondemote: u64,
    #[prost(fixed64, tag = "11")]
    pub goal_fx: u64,
    #[prost(fixed64, tag = "12")]
    pub medal: u64,
    #[prost(fixed64, tag = "13")]
    pub pattern: u64,
    #[prost(fixed64, tag = "14")]
    pub pattern_body: u64,
    #[prost(fixed64, tag = "15")]
    pub pip: u64,
    #[prost(fixed64, tag = "16")]
    pub tag: u64,
    #[prost(fixed64, tag = "17")]
    pub tint: u64,
    #[prost(fixed64, tag = "18")]
    pub tint_alignment_a: u64,
    #[prost(fixed64, tag = "19")]
    pub tint_alignment_b: u64,
    #[prost(fixed64, tag = "20")]
    pub tint_body: u64,
    #[prost(fixed64, tag = "21")]
    pub title: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadoutEntry {
    #[prost(fixed64, tag = "1")]
    pub bodytype: u64,
    #[prost(uint32, tag = "2")]
    pub teamid: u32,
    #[prost(uint32, tag = "3")]
    pub airole: u32,
    #[prost(fixed64, tag = "4")]
    pub xf: u64,
    #[prost(message, optional, tag = "5")]
    pub loadout: Option<LoadoutSlot>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LoadoutInstance {
    #[prost(fixed64, tag = "1")]
    pub instance_name: u64,
    #[prost(message, repeated, tag = "2")]
    pub items: Vec<LoadoutItem>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GameServerSaveLoadoutMessage {
    #[prost(string, tag = "1")]
    pub lobby_session_id: String,
    #[prost(string, tag = "2")]
    pub entrant_id: String,
    #[prost(int32, tag = "3")]
    pub loadout_slot: i32,
    #[prost(int32, tag = "4")]
    pub jersey_number: i32,
    #[prost(message, repeated, tag = "5")]
    pub loadout_instances: Vec<LoadoutInstance>,
    #[prost(message, optional, tag = "6")]
    pub loadout_entry: Option<LoadoutEntry>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbyEntrantsConnectedMessage {
    #[prost(string, tag = "1")]
    pub lobby_session_id: String,
    #[prost(string, repeated, tag = "2")]
    pub entrant_ids: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbyEntrantsAcceptMessage {
    #[prost(string, repeated, tag = "1")]
    pub entrant_ids: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbyEntrantsRejectMessage {
    #[prost(string, repeated, tag = "1")]
    pub entrant_ids: Vec<String>,
    #[prost(int32, tag = "2")]
    pub code: i32,
}

/// Why an entrant was refused or removed (shared by both messages).
macro_rules! entrant_code {
    () => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
        #[repr(i32)]
        pub enum Code {
            InternalUnspecified = 0,
            BadRequest = 1,
            Timeout = 2,
            DuplicateUserId = 3,
            LobbyLocked = 4,
            LobbyFull = 5,
            LobbyEnding = 6,
            Kicked = 7,
            Disconnected = 8,
            Inactive = 9,
        }
    };
}

pub mod lobby_entrants_reject_message {
    entrant_code!();
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbyEntrantRemovedMessage {
    #[prost(string, tag = "1")]
    pub lobby_session_id: String,
    #[prost(string, tag = "2")]
    pub entrant_id: String,
    #[prost(int32, tag = "3")]
    pub code: i32,
}

pub mod lobby_entrant_removed_message {
    entrant_code!();
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct LobbySessionCreateMessage {
    #[prost(string, tag = "1")]
    pub lobby_session_id: String,
    #[prost(int32, tag = "2")]
    pub lobby_type: i32,
    #[prost(string, tag = "3")]
    pub group_id: String,
    #[prost(int32, tag = "4")]
    pub max_entrants: i32,
    #[prost(string, tag = "5")]
    pub settings_json: String,
    #[prost(string, repeated, tag = "6")]
    pub features: Vec<String>,
    #[prost(uint32, tag = "7")]
    pub time_step_usecs: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct XPlatformId {
    #[prost(string, tag = "1")]
    pub id: String,
}

/// A player's session keys and endpoint, handed to the game server.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SnsLobbySessionSuccessV5Message {
    #[prost(fixed64, tag = "1")]
    pub game_mode: u64,
    #[prost(string, tag = "2")]
    pub lobby_id: String,
    #[prost(string, tag = "3")]
    pub group_id: String,
    #[prost(string, tag = "4")]
    pub endpoint: String,
    #[prost(int32, tag = "5")]
    pub team_index: i32,
    #[prost(uint32, tag = "6")]
    pub user_slot: u32,
    #[prost(uint32, tag = "7")]
    pub flags32: u32,
    #[prost(uint32, tag = "8")]
    pub session_flags: u32,
    #[prost(uint64, tag = "9")]
    pub server_encoder_flags: u64,
    #[prost(uint64, tag = "10")]
    pub client_encoder_flags: u64,
    #[prost(uint64, tag = "11")]
    pub server_sequence_id: u64,
    #[prost(bytes = "vec", tag = "12")]
    pub server_mac_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "13")]
    pub server_enc_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "14")]
    pub server_random_key: Vec<u8>,
    #[prost(uint64, tag = "15")]
    pub client_sequence_id: u64,
    #[prost(bytes = "vec", tag = "16")]
    pub client_mac_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "17")]
    pub client_enc_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "18")]
    pub client_random_key: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SnsRemoteLogSetV3Message {
    #[prost(message, optional, tag = "1")]
    pub user_id: Option<XPlatformId>,
    #[prost(uint64, tag = "2")]
    pub log_level: u64,
    #[prost(string, repeated, tag = "3")]
    pub logs: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SnsUserServerProfileUpdateRequestMessage {
    #[prost(message, optional, tag = "1")]
    pub user_id: Option<XPlatformId>,
    #[prost(string, tag = "2")]
    pub payload: String,
}
