//! Golden vectors produced by nakama3's Go encoder (`server/evr`), the
//! reference implementation the final-build client is known to accept.

use super::*;

fn golden(name: &str) -> Vec<u8> {
    let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/golden_vectors.json")).unwrap();
    hex::decode(v[name].as_str().unwrap()).unwrap()
}
fn sid() -> Uuid {
    "2a4a1b12-2bf9-11ef-8dd5-66d3ff8a653b".parse().unwrap()
}
fn lid() -> Uuid {
    "11223344-5566-7788-99aa-bbccddeeff00".parse().unwrap()
}
fn gid() -> Uuid {
    "90dd4db5-b5dd-4655-839e-fdbe5f4bc0bf".parse().unwrap()
}
const XP: XpId = XpId { platform: 4, account: 123412341234 };

fn one(buf: &[u8]) -> (u64, Vec<u8>) {
    let m = split(buf).unwrap();
    assert_eq!(m.len(), 1);
    (m[0].0, m[0].1.to_vec())
}

#[test]
fn symbols_match_reference() {
    let v: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/symbols.json")).unwrap();
    for (name, hex) in v.as_object().unwrap() {
        if name.starts_with("SNS") {
            continue; // message type names hash differently (case-folded)
        }
        let want = u64::from_str_radix(hex.as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
        assert_eq!(symbol(name), want, "{name}");
    }
}

#[test]
fn server_messages_byte_identical() {
    let check = |name: &str, f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = Vec::new();
        f(&mut b);
        assert_eq!(hex::encode(&b), hex::encode(golden(name)), "{name}");
    };
    check("login_success", &|b| login_success(b, sid(), XP));
    check("login_failure", &|b| login_failure(b, XP, 400, "bad"));
    check("unrequire", &|b| unrequire(b));
    check("matchmaker_status", &|b| matchmaker_status(b));
    check("entrants_v3", &|b| entrants_v3(b, XP, sid(), 1));
    // failure_v4 ends in an i64 expiry; Go writes its zero time (year 1), we write 0.
    let mut b = Vec::new();
    session_failure_v4(&mut b, symbol("echo_arena"), gid(), fail::SERVER_FIND_FAILED, "no servers");
    let g = golden("failure_v4");
    assert_eq!(b.len(), g.len());
    assert_eq!(hex::encode(&b[..b.len() - 8]), hex::encode(&g[..g.len() - 8]));
    check("update_profile_success", &|b| update_profile_success(b, XP));
}

#[test]
fn session_success_v5_byte_identical() {
    let k = |base: u8| -> [u8; 32] { std::array::from_fn(|i| base.wrapping_add(i as u8)) };
    let s = SessionSuccess {
        mode: symbol("echo_arena"),
        lobby_id: lid(),
        group_id: gid(),
        internal_ip: "192.168.1.2".parse().unwrap(),
        external_ip: "1.2.3.4".parse().unwrap(),
        port: 6792,
        team: 1,
        session_flags: 0,
        server_seq: 0x0102030405060708,
        server_mac: k(0),
        server_enc: k(0x20),
        server_rnd: k(0x40),
        client_seq: 0x1112131415161718,
        client_mac: k(0x60),
        client_enc: k(0x80),
        client_rnd: k(0xa0),
    };
    let mut b = Vec::new();
    s.encode_v5(&mut b);
    assert_eq!(hex::encode(&b), hex::encode(golden("session_success_v5")));
    let (_, p) = one(&b);
    let d = decode_session_success_v5(&p).unwrap();
    assert_eq!(d.endpoint_string(), "192.168.1.2:1.2.3.4:6792");
    assert_eq!(d.client_rnd, s.client_rnd);
    assert_eq!(d.server_seq, s.server_seq);
}

#[test]
fn compressed_payloads_round_trip() {
    let json = br#"{"type":"main_menu","id":"main_menu"}"#;
    let mut b = Vec::new();
    config_success(&mut b, "main_menu", "main_menu", json).unwrap();
    let (s, p) = one(&b);
    assert_eq!(s, sym::CONFIG_SUCCESS);
    let mut r = Reader::new(&p);
    assert_eq!(r.u64().unwrap(), symbol("main_menu"));
    r.u64().unwrap();
    assert_eq!(unzstd_json(&mut r).unwrap()["id"], "main_menu");

    let mut b = Vec::new();
    game_settings(&mut b, br#"{"env":"live"}"#).unwrap();
    let (_, p) = one(&b);
    assert_eq!(unzlib_json(&mut Reader::new(&p)).unwrap()["env"], "live");
}

#[test]
fn decodes_client_requests() {
    let dec = |name: &str| {
        let (s, p) = one(&golden(name));
        ClientMessage::decode(s, &p).unwrap()
    };
    match dec("login_request") {
        ClientMessage::LoginRequest { xpid, profile, .. } => {
            assert_eq!(xpid, XP);
            assert_eq!(profile["displayname"], "Tester");
        }
        m => panic!("{m:?}"),
    }
    assert!(matches!(dec("config_request"), ClientMessage::ConfigRequest { ref id, .. } if id == "main_menu"));
    assert!(matches!(dec("document_request"), ClientMessage::DocumentRequest { ref kind, ref lang } if kind == "eula" && lang == "en"));
    assert!(matches!(dec("profile_request"), ClientMessage::LoggedInProfileRequest { session, xpid } if session == sid() && xpid == XP));
    assert!(matches!(dec("channel_info_request"), ClientMessage::ChannelInfoRequest));
    assert!(matches!(dec("pending_cancel"), ClientMessage::PendingSessionCancel { session } if session == sid()));
    match dec("player_sessions_request") {
        ClientMessage::PlayerSessionsRequest { lobby_id, players, .. } => {
            assert_eq!(lobby_id, lid());
            assert_eq!(players, vec![XP, XpId { platform: 4, account: 5 }]);
        }
        m => panic!("{m:?}"),
    }
    match dec("create_request") {
        ClientMessage::Lobby(r) => {
            assert!(matches!(r.kind, LobbyRequestKind::Create { lobby_type: 1, .. }));
            assert_eq!(r.mode, symbol("echo_arena_private"));
            assert_eq!(r.group_id, gid());
            assert_eq!(r.settings.appid, "1369078409873402");
            assert_eq!(r.entrants[0].xpid, XP);
            assert_eq!(r.entrants[0].role, 0);
        }
        m => panic!("{m:?}"),
    }
}

/// Go's encoder omits the alignment padding its own decoder (and the game
/// client) uses for find/join requests; re-insert it to get the client layout.
fn padded(name: &str, at: usize, n: usize) -> (u64, Vec<u8>) {
    let (s, p) = one(&golden(name));
    let mut q = p[..at].to_vec();
    q.extend(std::iter::repeat_n(0, n));
    q.extend_from_slice(&p[at..]);
    (s, q)
}

#[test]
fn decodes_find_and_join() {
    // find: 4 symbols, login session, u8 count, u32 flags | 3 pad | current lobby, group.
    for name in ["find_request", "find_request_team"] {
        let (s, p) = padded(name, 32 + 16 + 5, 3);
        match ClientMessage::decode(s, &p).unwrap() {
            ClientMessage::Lobby(r) => {
                assert!(matches!(r.kind, LobbyRequestKind::Find { .. }), "{name}");
                assert_eq!(r.group_id, gid(), "{name}");
                assert_eq!(r.entrants.len(), 1, "{name}");
                assert_eq!(r.entrants[0].xpid, XP, "{name}");
                if name == "find_request_team" {
                    assert!(matches!(r.kind, LobbyRequestKind::Find { current_lobby } if current_lobby == lid()));
                    assert_eq!(r.mode, symbol("echo_arena"));
                    assert!(r.cross_play);
                    assert_eq!(r.entrants[0].role, 1);
                }
            }
            m => panic!("{m:?}"),
        }
    }
    // join: lobby id, version lock, platform, login session, u32 flags | 4 pad.
    let (s, p) = padded("join_request", 16 + 16 + 16 + 4, 4);
    match ClientMessage::decode(s, &p).unwrap() {
        ClientMessage::Lobby(r) => {
            assert!(matches!(r.kind, LobbyRequestKind::Join { lobby_id, moderate: None } if lobby_id == lid()));
            assert_eq!(r.entrants.len(), 1);
            assert_eq!(r.entrants[0].xpid, XP);
        }
        m => panic!("{m:?}"),
    }
}

#[test]
fn xpid_tokens() {
    assert_eq!(XP.token(), "OVR_ORG-123412341234");
    assert_eq!(XpId::parse("OVR_ORG-123412341234"), Some(XP));
    assert_eq!(XpId::parse("DMO-5"), Some(XpId { platform: 7, account: 5 }));
}
