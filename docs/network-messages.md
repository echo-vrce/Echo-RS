# Echo VR network messages

Every message type in the native recordings under `captures/` (36 recordings, both directions), named.
Names come from the recordings themselves, the game symbol dump (`echo-symbols_2026-10-19.json`, message ids are the game's matrix hash of the type name, service ids its CRC symbol hash) and `src/sns.rs`.

- **s2c / c2s**: how many times it was seen from the server / from the client, all recordings together.
- **Sizes**: payload lengths seen (first few).
- **Our code**: the server names or handles it.
- **Sent by ours**: our server sent it in the 2026-11-24 live log (`dist/log.txt`, two combat matches).


## Game

| Name | Id | s2c | c2s | Sizes | Our code | Sent by ours |
|---|---|---:|---:|---|---|---|
| `SR15ForcePushPlayer` | `0x1e7cda39027bb6cc` | 45873 | 25421 | 16 | yes | yes |
| `SR15NetAbilityActivate` | `0x06604436ae89aae0` | 747 | 181 | 8 | yes | yes |
| `SR15NetAbilityCooldown` | `0xc4ad1c91b858ef08` | 728 | 90 | 8 | yes | yes |
| `SR15NetActivateExplosion` | `0x36ad7f161cfba5a2` | 0 | 246 | 40 | yes |  |
| `SR15NetAutoTargetFireBullet` | `0x95bcc1e3fec575b2` | 218 | 882 | 48 | yes | yes |
| `SR15NetBalanceSettingsClientReady` | `0x10ec6147d112e1e4` | 2 | 104 | 1 | yes |  |
| `SR15NetBalanceSettingsMassUpdate` | `0xe9a4b52cba8b27e7` | 104 | 2 | 3334, 4171, 4653, 4762 | yes | yes |
| `SR15NetBitFieldRequest` | `0x06bc9290c472273a` | 2 | 78 | 8 | yes |  |
| `SR15NetBitFieldResponse` | `0x13f6ad878f60cb05` | 78 | 2 | 968 | yes | yes |
| `SR15NetCaptureVolumeReset` | `0x3125c641e7b157bd` | 12 | 0 | 16 | yes |  |
| `SR15NetCaptureVolumeUpdate` | `0x9fd63bf21c96ef8e` | 36421 | 0 | 40 | yes | yes |
| `SR15NetClientPingResponseMsg` | `0x67232ca3535df81c` | 21589 | 2600518 | 48 | yes |  |
| `SR15NetComponentEvent` | `0x6e54d0d298bffb17` | 297271 | 5269 | 24 | yes | yes |
| `SR15NetCooldownActive` | `0x39a7e87c23af7ac6` | 370 | 0 | 16 | yes | yes |
| `SR15NetCooldownRefund` | `0x795fa17c1eb9ec3a` | 340 | 0 | 16 | yes | yes |
| `SR15NetCurrentLoadoutRequest` | `0xc66ca34bb55fdc28` | 133 | 725 | 4 | yes | yes |
| `SR15NetCurrentLoadoutResponse` | `0xa52712ec66b2db48` | 1080 | 142 | 388, 404, 452 | yes | yes |
| `SR15NetDamageableBulletCollision` | `0xcf5e411193b929f3` | 0 | 2480 | 64 | yes |  |
| `SR15NetDamageableDamageDealt` | `0x534e3b3f0f17e56b` | 0 | 8 | 32 | yes |  |
| `SR15NetDamageableDealDamage` | `0xe669be764c8a1db6` | 2811 | 0 | 24 | yes | yes |
| `SR15NetDamageableHealthBroadcast` | `0x97bbbd245d467354` | 106527 | 0 | 64, 120, 176, 344 | yes | yes |
| `SR15NetDamageableLocallyTrackedDamage` | `0x690bb5171245edf2` | 751 | 0 | 40 | yes | yes |
| `SR15NetDamageablePlayerKilled` | `0xba9db3f79fa8a511` | 277 | 0 | 24 | yes | yes |
| `SR15NetDamageablePlayerRespawn` | `0xd2b758303e284785` | 285 | 37 | 16 | yes | yes |
| `SR15NetDamageablePlayerRespawnComplete` | `0xcad5776e4afc09d2` | 0 | 285 | 16 | yes | yes |
| `SR15NetDamageableShieldRepaired` | `0x87e6b4fa40a91118` | 56 | 0 | 16 | yes | yes |
| `SR15NetExplosionNothingHit` | `0x7da3136ff17537ed` | 16689 | 0 | 24 | yes | yes |
| `SR15NetExplosionPlayerDamage` | `0xc9bf5b1dbb888def` | 553 | 0 | 48 | yes | yes |
| `SR15NetGameActivateHealBubbleMsg` | `0xc472a1be2ea5776e` | 136 | 28 | 24 | yes |  |
| `SR15NetGameDepositMagazine` | `0x56e20c9ef59659ef` | 36 | 7 | 16 | yes | yes |
| `SR15NetGameDestroyMagazine` | `0x94ff8d9ddf80936d` | 390 | 0 | 8 | yes | yes |
| `SR15NetGameFireGun` | `0xe22470ae4641ee83` | 28912 | 5299 | 64 | yes | yes |
| `SR15NetGameplaySensorTriggered` | `0x889bac7502b461f5` | 0 | 8 | 12 | no |  |
| `SR15NetGameplayTeleportPlayer` | `0x4d7fcb464e548b84` | 35389 | 3464 | 36 | yes | yes |
| `SR15NetGameplayTeleportPlayerRequest` | `0x0fbac5d994dde2e8` | 3277 | 35701 | 36 | yes |  |
| `SR15NetGameSetMagazineState` | `0x5186cb60ac8ef1a2` | 1501 | 361 | 24 | yes | yes |
| `SR15NetGameSpawnMagazine` | `0x9cd4195be16305e5` | 383 | 97 | 40 | yes | yes |
| `SR15NetHealBubbleParamUpdate` | `0x40b2dd75fc72e38f` | 1259 | 400 | 56 | yes | yes |
| `SR15NetIdsRequest` | `0xc4fe7b57caab27e2` | 8 | 315 | 8 | yes |  |
| `SR15NetIdsResponse` | `0x8f8af081af21cbe7` | 314 | 8 | 138, 188, 218, 278 | yes | yes |
| `SR15NetItemEquippedNotification` | `0xed0fba849badc7bf` | 37 | 48 | 32 | yes |  |
| `SR15NetLoadoutNumberNotification` | `0x80af66bf1e67b474` | 120 | 59 | 12 | yes | yes |
| `SR15NetNewUnlocks` | `0x187008fe3940a327` | 92 | 2 | 1817, 1841, 1849, 1889 | yes |  |
| `SR15NetPayloadPosStateUpdateReliable` | `0x3ef8a408b935f46b` | 1593 | 0 | 4 | yes | yes |
| `SR15NetPayloadTrackpointReachedReliable` | `0x2b5e8480b5fbd1e3` | 1399 | 0 | 4 | yes | yes |
| `SR15NetPayloadUpdatedAutodriveReliable` | `0x66fac59368547aee` | 13 | 0 | 8 | yes | yes |
| `SR15NetPayloadUpdateMiscStateReliable` | `0x17d4cde80c164a9a` | 916 | 0 | 8 | yes | yes |
| `SR15NetPhysicsSimStateEvent` | `0x8f62adba23f15fe0` | 5284660 | 1249385 | 64, 96, 136, 168 | yes | yes |
| `SR15NetPlayEmote` | `0x7c63c5877b0eee97` | 287 | 65 | 16 | yes | yes |
| `SR15NetPlayerEquipmentUpdate` | `0x71c59625b07711b9` | 3715 | 900 | 56 | yes | yes |
| `SR15NetPlayerStateMsg` | `0xc6bb60e8d9e9a3d3` | 3297176 | 921268 | 64 | yes | yes |
| `SR15NetPooledActorCreate` | `0xe82df95823db043b` | 1289 | 468 | 96, 160 | yes | yes |
| `SR15NetPooledActorDestroy` | `0xf477a3ea01bf0489` | 1716 | 121 | 16 | yes | yes |
| `SR15NetReliablePostScoreUpdate` | `0xb9f8831ab6e068a9` | 137 | 10 | 28 | yes |  |
| `SR15NetReliableStatUpdate` | `0x5d84b0c2d4bb6cfc` | 22364 | 12 | 7, 49, 62, 63 | yes | yes |
| `SR15NetReliableTeamStatUpdate` | `0x7ba444f7d419b1a2` | 8512 | 8 | 142, 162, 242 | yes | yes |
| `SR15NetReplicatedClientReady` | `0xa026b3042add4e14` | 2 | 104 | 16 | yes |  |
| `SR15NetReplicatedDeltaAck` | `0xd0aead470b6648b8` | 4412 | 2015003 | 24 | yes |  |
| `SR15NetReplicatedDeltaUnreliableVars` | `0x2ded551babc46dbf` | 2353422 | 4431 | 46, 62, 126, 5054 | yes | yes |
| `SR15NetSaveLoadoutRequest` | `0xa3749626d6d20ef7` | 0 | 37 | 4 | yes |  |
| `SR15NetSensorPing` | `0x2050928ad5a3b7d4` | 8 | 32 | 16 | no |  |
| `SR15NetServerFrameMsg` | `0xc57ec38913be8582` | 3084690 | 21590 | 16 | yes | yes |
| `SR15NetServerInfo` | `0xfa563ae00ebff2ba` | 111 | 2 | 96 | yes | yes |
| `SR15NetSetBits` | `0xb043bde1197c11f1` | 6461 | 1420 | 8 | yes | yes |
| `SR15NetSetOwner` | `0x4a2953dc4d5fbaed` | 0 | 184 | 8 | yes |  |
| `SR15NetTierRewardMsg` | `0xb3acdcc23a081c92` | 14 | 0 | 32 | yes |  |
| `SR15NetTopAwardsMsg` | `0xf28582c141e30960` | 35 | 0 | 73, 145, 217, 289 | yes |  |
| `SR15NetTouchOwnershipDeclined` | `0x8255a3b48e69fd70` | 16 | 0 | 32 | yes |  |
| `SR15NetTouchOwnershipGranted` | `0x84745d47a4b31f27` | 612 | 4 | 32 | yes |  |
| `SR15NetTouchOwnershipTransferMailbox` | `0x60fd8780fe4017f5` | 5 | 705 | 48 | yes |  |
| `SR15NetTouchOwnershipTransferReliable` | `0x6f248c06a631d7cb` | 5 | 705 | 48 | yes |  |
| `SR15NetTouchStateEvent` | `0x6d9a367ee21a5642` | 605232 | 135702 | 16 | yes | yes |
| `SR15NetUIDecSettingMsg` | `0x1ef9c84f7285f216` | 0 | 49 | 24 | yes |  |
| `SR15NetUIIncSettingMsg` | `0xe7ece98b6a0ac2b6` | 0 | 48 | 24 | yes |  |
| `SR15NetUIUserChangedSettings` | `0x032d1569cbfd89bd` | 97 | 0 | 32 | yes |  |
| `SR15NetUserAssignTeamRequest` | `0xab60f5197bf1f96b` | 0 | 16 | 6 | yes |  |
| `SR15NetUserAssignTeamResponse` | `0x212d30f7f0093a97` | 16 | 0 | 2 | yes |  |
| `SR15NetUserPingEstMsg` | `0xf85fd7296f268b1b` | 9488893 | 112332 | 24 | yes | yes |
| `SR15NetUserVoipEvent` | `0x599b9fd3de2fa94f` | 1881002 | 441210 | 28, 36, 40, 63 | yes | yes |
| `SR15StunMsg` | `0x1feb495968559bc3` | 22 | 0 | 16 | yes |  |
| `SR15StunPlayerMsg` | `0xccd9ed2d3125ff67` | 202 | 31 | 16 | yes | yes |

## Other

| Name | Id | s2c | c2s | Sizes | Our code | Sent by ours |
|---|---|---:|---:|---|---|---|
| `(unnamed: seen once, 16 zero bytes)` | `0xe23415177fd5a971` | 1 | 0 | 16 | no |  |
| `?` | `0x9ee5107d9e29fd63` | 0 | 135 | 26, 40, 42, 173 | yes |  |

## Services (login, profile, matchmaking, social)

| Name | Id | s2c | c2s | Sizes | Our code | Sent by ours |
|---|---|---:|---:|---|---|---|
| `(unnamed: social presence/activity status, carries "Public Arena Match")` | `0xbdd8dd00c5e97a63` | 2016 | 103 | 34, 46, 52, 53 | no |  |
| `SNS CHANNEL_INFO_RESPONSE` | `0x6c8f16cd9f8964c5` | 19 | 8 | 276, 299 | yes |  |
| `SNS CONFIG_SUCCESS` | `0xb9cdaf586f7bd012` | 76 | 32 | 228, 444, 507, 510 | yes |  |
| `SNS DOCUMENT_SUCCESS` | `0xd07ffd782fb7b509` | 19 | 8 | 243, 253, 259, 275 | yes |  |
| `SNS ENTRANTS_V3` | `0xa1b9cae1f8588969` | 42 | 2 | 41 | yes |  |
| `SNS ER_PLAYERS_ACCEPTED` | `0x7777777777770600` | 0 | 55 | 17 | yes |  |
| `SNS ER_PLAYERS_REJECTED` | `0x7777777777770700` | 0 | 10 | 17 | yes |  |
| `SNS ER_REGISTRATION_SUCCESS` | `0xb57a31cdd0f6fedf` | 0 | 7 | 20 | yes |  |
| `SNS ER_START_SESSION` | `0x7777777777770000` | 0 | 7 | 148, 149 | yes |  |
| `SNS GAME_SETTINGS` | `0xed5be2c3632155f1` | 19 | 8 | 213, 219, 400 | yes |  |
| `SNS LOGGED_IN_PROFILE_SUCCESS` | `0xfb763a5037fc8d77` | 19 | 8 | 7741, 8011, 8018, 8036 | yes |  |
| `SNS LOGIN_SUCCESS` | `0xa5acc1a90d0cce47` | 19 | 8 | 32 | yes |  |
| `SNS MATCHMAKER_STATUS` | `0x8f28cf33dabfbecb` | 44 | 2 | 4 | yes |  |
| `SNS OTHER_PROFILE_FAILURE` | `0x1225133828150da3` | 13 | 0 | 42 | yes |  |
| `SNS OTHER_PROFILE_SUCCESS` | `0x1230073227050cb5` | 639 | 95 | 764, 1161, 1164, 1352 | yes |  |
| `SNS PING_REQUEST` | `0xfabf5f8719bfebf3` | 119 | 36 | 20, 32, 56, 104 | yes |  |
| `SNS RECONCILE_IAP_RESULT` | `0x0dabc24265508a82` | 18 | 2 | 84 | yes |  |
| `SNS SESSION_FAILURE_V4` | `0x4ae8365ebc45f96c` | 1 | 0 | 104 | yes |  |
| `SNS SESSION_SUCCESS_V5` | `0x6d4de3650ee3110f` | 42 | 58 | 280 | yes |  |
| `SNS UPDATE_PROFILE_SUCCESS` | `0xf25491d001cef757` | 189 | 10 | 16 | yes |  |
| `SNS USER_SERVER_PROFILE_UPDATE_SUCCESS` | `0xd299785ba56b9c75` | 0 | 64 | 16 | yes |  |
| `SNSFriendInviteNotify` | `0xca09b0b36bd981b7` | 1 | 0 | 16 | no |  |
| `SNSFriendListResponse` | `0xa78aeb2a4e89b10b` | 179 | 8 | 32 | no |  |
| `SNSFriendStatusNotify` | `0x26a19dc4d2d5579d` | 2070 | 99 | 24 | no |  |
| `SNSLobbyAcceptPlayersFailurev2` | `0xed9a4b86f8f3640a` | 0 | 10 | 17 | no |  |
| `SNSLobbyAcceptPlayersSuccessv2` | `0xfcba8f2834f8de40` | 0 | 62 | 1, 17 | no |  |
| `SNSLobbyEntrantAdded` | `0x299283a7141dbce2` | 132 | 68 | 8 | no |  |
| `SNSLobbyEntrantDataChanged` | `0x8f05b6735e916658` | 1759 | 615 | 16, 18, 24, 32 | yes | yes |
| `SNSLobbyEntrantRemoved` | `0x39156563932117d6` | 335 | 51 | 16 | yes | yes |
| `SNSLobbyEntrantStatev2` | `0xeb6218834b3fa648` | 4422 | 1083 | 328, 373, 376, 396 | yes | yes |
| `SNSLobbyEntrantTeamChanged` | `0xceb8759c36184b82` | 60 | 2 | 16 | yes | yes |
| `SNSLobbyHostDisconnected` | `0x9d73acf6ad59040d` | 2 | 0 | 1 | no |  |
| `SNSLobbyHostState` | `0x174ea9b31846fe3a` | 56 | 2 | 4 | yes |  |
| `SNSLobbyJoinAcceptedv2` | `0xb23e3c1167ec7a79` | 56 | 2 | 33 | yes |  |
| `SNSLobbyJoinComplete` | `0x65b2feba429e1b02` | 69 | 0 | 1 | yes | yes |
| `SNSLobbyJoinRequestv4` | `0xb4d724e8564bfe88` | 2 | 124 | 41 | yes |  |
| `SNSLobbyJoinSetup` | `0xb3e2b4258ff860e4` | 56 | 2 | 93, 101, 117 | yes |  |
| `SNSLobbyLockEntrants` | `0x2dd64ecd5b5f421c` | 41 | 6 | 1 | no |  |
| `SNSLobbyMatchmakerStatusReceived` | `0xce66e4786f0b118a` | 44 | 2 | 4 | no |  |
| `SNSLobbyOwnerDataChanged` | `0xadc58c2466ed01b6` | 302 | 128 | 3, 9, 11, 17 | no |  |
| `SNSLobbyOwnerState` | `0x2068c0f03b4b8f25` | 824 | 130 | 425, 449, 473, 481 | yes | yes |
| `SNSLobbyRegistrationSuccess` | `0x0fef8efec97a3b98` | 0 | 7 | 20 | no |  |
| `SNSLobbyRemovingEntrant` | `0xf75c4972e6267c4e` | 90 | 51 | 16 | no |  |
| `SNSLobbySessionFailurev4` | `0x4554f516a438c623` | 1 | 0 | 104 | no |  |
| `SNSLobbySessionStarting` | `0x233e6e7e3a13babc` | 0 | 7 | 41 | no |  |
| `SNSLobbySessionSuccessv5` | `0x83e96504a9fc81c6` | 42 | 58 | 280 | no |  |
| `SNSLobbyStartSessionv4` | `0x096101c684e7f325` | 0 | 7 | 148, 149 | no |  |
| `SNSLobbyUnlockEntrants` | `0xfc250b496eddbfa7` | 10 | 2 | 1 | no |  |
| `SNSLobbyUpdatePings` | `0x03e01d2bd46a86c0` | 24761 | 160 | 25, 33 | yes | yes |
| `SNSPartyInviteListResponse` | `0x685a5fb8447b1155` | 19 | 1 | 4 | no |  |
| `SNSPartyInviteNotify` | `0x218f721f09026dab` | 87 | 0 | 16 | no |  |
| `SNSRefreshProfileFromServer` | `0x06877b856e8d785f` | 118 | 2 | 1262, 1744, 1867, 4821 | yes | yes |

## Transport

| Name | Id | s2c | c2s | Sizes | Our code | Sent by ours |
|---|---|---:|---:|---|---|---|
| `SBroadcasterConnectEvent` | `0x5069b99c8b15cbd8` | 116 | 155 | 16 | yes |  |
| `SBroadcasterConnectFailedEvent` | `0x0fed6de7c416da1f` | 42 | 2 | 1 | no |  |
| `SBroadcasterDisconnectEvent` | `0xe24e16d850752b9a` | 82 | 85 | 0, 1 | yes |  |
| `SBroadcasterEnableCompressionEvent_1` | `0x4f513db80cc3e10a` | 71 | 56 | 0 | yes | yes |
| `SBroadcasterMsgEncodingFailEvent` | `0x59e4c5ea6e01083b` | 12216 | 5344 | 8, 16, 24, 32 | yes |  |
| `SBroadcasterMsgEncodingSupportEvent_1` | `0x1175ae5766f221aa` | 113 | 113 | 0 | yes |  |
| `SBroadcasterPingUpdate` | `0x014c925361833bf6` | 15144 | 2760 | 0 | yes | yes |
| `SBroadcasterReliableBatchEvent` | `0x2cd38a848b5b9c73` | 97 | 2 | 350, 824, 2633, 2727 | yes |  |
| `STcpConnectionEstablishedEvent` | `0xf2cbbc7e6ef8b056` | 48 | 17 | 8 | no |  |
| `STcpPeerConnectEvent` | `0x210db58b9c5262c0` | 68 | 27 | 1 | no |  |
| `STcpPeerDisconnectEvent` | `0xf92978e408c610c8` | 24 | 16 | 1 | no |  |

## Replicated vars found missing (2026-11-24)

Native sends each player's gun heat as a replicated var (`SR15NetReplicatedDeltaUnreliableVars`), one key per player slot, value `[heat f32][0 u16][slot u16]`. Keys for slots 0-9 are in `HEAT_KEYS` (src/engine/gear.rs); slots 10 and up are not known. Ours sends them from 0.2.8.

Two flags native sets near the end of a match are not sent by ours: `ad9af64796d5b9e4` (Fission, Combustion) and `846bc1fc488331e4` (Fission, Gauss, Dyson), plus `425af56662f0d670` (Gauss). Each flips 0/1 a few times; their value carries a handle in the upper bytes. Level-script switches, names unknown.
