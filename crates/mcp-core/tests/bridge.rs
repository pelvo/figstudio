use figstudio_fig::Guid;
use figstudio_mcp_core::bridge::{
    ClientMessage, PROTOCOL_VERSION, ServerMessage, TabState, guid_string, parse_guid,
};
use serde::{Serialize, de::DeserializeOwned};

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(message: T) {
    let json = serde_json::to_string(&message).unwrap();
    assert_eq!(serde_json::from_str::<T>(&json).unwrap(), message);
    assert!(json.contains("\"type\""));
}

#[test]
fn every_frame_roundtrips() {
    roundtrip(ClientMessage::Hello {
        token: "pairing".into(),
        version: PROTOCOL_VERSION,
    });
    roundtrip(ClientMessage::Open {
        tab: 7,
        name: "structure.fig".into(),
        size: 20,
    });
    roundtrip(ClientMessage::End { tab: 7 });
    roundtrip(ClientMessage::State {
        tabs: vec![TabState {
            tab: 7,
            page: Some("0:1".into()),
            selection: Some("1:2".into()),
            viewport: Some([1., 2., 3., 4.]),
        }],
        active: Some(7),
    });
    roundtrip(ClientMessage::Close { tab: 7 });
    roundtrip(ClientMessage::Reply { id: 9, error: None });
    roundtrip(ClientMessage::Reply {
        id: 9,
        error: Some("not found".into()),
    });
    roundtrip(ServerMessage::Welcome {
        version: PROTOCOL_VERSION,
    });
    roundtrip(ServerMessage::Error {
        message: "invalid upload".into(),
    });
    roundtrip(ServerMessage::Reveal {
        id: 9,
        tab: 7,
        node: "1:2".into(),
    });
}

#[test]
fn rejects_unknown_or_malformed_frames() {
    assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<ServerMessage>(r#"{"type":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"open","tab":-1}"#).is_err());
}

#[test]
fn wire_ids_are_only_guids() {
    assert_eq!(parse_guid(&guid_string(Guid(1, 2))).unwrap(), Guid(1, 2));
    for bad in ["", "1-2", "I1:2;3:4", "1:2:3", "-1:2", "4294967296:2"] {
        assert!(parse_guid(bad).is_err(), "accepted {bad}");
    }
}
