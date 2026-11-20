//! Live collaboration messages (`craft-live/1`) between a hosting page and the app, and the app's
//! status reports back. Unlike the file messages in the crate root these are unprefixed
//! (`type: "live"`, `"live-state"`, `"ready"`): the Nextcloud page speaks them to every app.
//! Pure and tested natively; [`crate::web`] (wasm32) moves them through `postMessage`.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::file_name;

/// The protocol a `live` message must name.
pub const LIVE_PROTO: &str = "craft-live/1";
/// `type` of the host's message that hands the app a ticket and a room.
pub const LIVE_TYPE: &str = "live";
/// `type` of the app's status report to the host.
pub const LIVE_STATE_TYPE: &str = "live-state";
/// The unprefixed `ready` an app posts so a live host knows to send `live`.
pub const READY_JSON: &str = r#"{"type":"ready"}"#;

/// Longest ticket taken, in bytes.
pub const MAX_TICKET_BYTES: usize = 4096;
/// Longest URL, room or reason taken, in bytes.
pub const MAX_FIELD_BYTES: usize = 2048;
/// Most peers taken in one report.
pub const MAX_PEERS: usize = 256;

/// Why a message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveError {
    /// Not JSON, or not an object.
    Malformed(String),
    /// The `type` is not the one expected.
    WrongType,
    /// A required string field is absent, not a string, or empty.
    MissingField(&'static str),
    /// A `proto` this build does not speak (carries it, cut short).
    UnsupportedProto(String),
    /// `relayUrl` is not an absolute `ws://` or `wss://` URL.
    BadRelayUrl,
    /// A field is longer than allowed.
    TooLong(&'static str),
    /// `state` is not one of the known states.
    UnknownState,
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "malformed live message: {why}"),
            Self::WrongType => f.write_str("not a live message"),
            Self::MissingField(name) => write!(f, "live message lacks {name}"),
            Self::UnsupportedProto(proto) => write!(f, "unsupported live protocol {proto:?}"),
            Self::BadRelayUrl => f.write_str("relayUrl must be an absolute ws:// or wss:// URL"),
            Self::TooLong(name) => write!(f, "live message field {name} is too long"),
            Self::UnknownState => f.write_str("unknown live state"),
        }
    }
}

impl std::error::Error for LiveError {}

/// The host's grant to join a room. `Debug` hides the ticket.
#[derive(Clone, PartialEq, Eq)]
pub struct LiveMessage {
    ticket: String,
    pub relay_url: String,
    pub room: String,
    pub file_url: String,
    /// Already cleaned ([`crate::file_name`]).
    pub file_name: String,
}

impl fmt::Debug for LiveMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveMessage")
            .field("ticket", &"<redacted>")
            .field("relay_url", &self.relay_url)
            .field("room", &self.room)
            .field("file_url", &self.file_url)
            .field("file_name", &self.file_name)
            .finish()
    }
}

fn object(json: &str) -> Result<Map<String, Value>, LiveError> {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(LiveError::Malformed("not an object".into())),
        // The parser's message can quote input; say only where.
        Err(e) => Err(LiveError::Malformed(format!("line {} column {}", e.line(), e.column()))),
    }
}

fn text<'a>(map: &'a Map<String, Value>, key: &'static str, max: usize) -> Result<&'a str, LiveError> {
    let value = map.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or(LiveError::MissingField(key))?;
    if value.len() > max { Err(LiveError::TooLong(key)) } else { Ok(value) }
}

impl LiveMessage {
    /// Read a `live` message from its JSON. Checks the `proto` first so a newer host gets a typed
    /// refusal, then every field.
    pub fn parse(json: &str) -> Result<Self, LiveError> {
        let map = object(json)?;
        if map.get("type").and_then(Value::as_str) != Some(LIVE_TYPE) {
            return Err(LiveError::WrongType);
        }
        let proto = text(&map, "proto", MAX_FIELD_BYTES)?;
        if proto != LIVE_PROTO {
            return Err(LiveError::UnsupportedProto(proto.chars().take(64).collect()));
        }
        let ticket = text(&map, "ticket", MAX_TICKET_BYTES)?.to_string();
        let relay_url = text(&map, "relayUrl", MAX_FIELD_BYTES)?;
        if !(relay_url.starts_with("ws://") || relay_url.starts_with("wss://")) || relay_url.chars().any(char::is_whitespace) {
            return Err(LiveError::BadRelayUrl);
        }
        let room = text(&map, "room", MAX_FIELD_BYTES)?.to_string();
        let file_url = text(&map, "fileUrl", MAX_FIELD_BYTES)?.to_string();
        let name = map.get("fileName").and_then(Value::as_str);
        Ok(Self { ticket, relay_url: relay_url.to_string(), room, file_url, file_name: file_name(name, "document") })
    }

    /// The ticket, for the connection only: never log it.
    pub fn ticket(&self) -> &str {
        &self.ticket
    }

    /// The message as the host sends it.
    pub fn to_json(&self) -> String {
        json!({
            "type": LIVE_TYPE, "ticket": self.ticket, "relayUrl": self.relay_url, "room": self.room,
            "fileUrl": self.file_url, "fileName": self.file_name, "proto": LIVE_PROTO,
        })
        .to_string()
    }
}

/// Where the live session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStatus {
    Connecting,
    Live,
    Readonly,
    Reconnecting,
    Refused,
}

impl LiveStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Live => "live",
            Self::Readonly => "readonly",
            Self::Reconnecting => "reconnecting",
            Self::Refused => "refused",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        [Self::Connecting, Self::Live, Self::Readonly, Self::Reconnecting, Self::Refused].into_iter().find(|v| v.as_str() == s)
    }
}

/// Someone else in the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    pub display: String,
}

/// The app's report to the host (`live-state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    pub state: LiveStatus,
    pub reason: Option<String>,
    pub saved_at: Option<String>,
    pub peers: Vec<Peer>,
}

impl LiveState {
    pub fn to_json(&self) -> String {
        let mut m = json!({
            "type": LIVE_STATE_TYPE, "state": self.state.as_str(),
            "peers": self.peers.iter().map(|p| json!({"id": p.id, "display": p.display})).collect::<Vec<_>>(),
        });
        if let Some(map) = m.as_object_mut() {
            if let Some(r) = &self.reason {
                map.insert("reason".into(), r.clone().into());
            }
            if let Some(s) = &self.saved_at {
                map.insert("savedAt".into(), s.clone().into());
            }
        }
        m.to_string()
    }

    pub fn parse(json: &str) -> Result<Self, LiveError> {
        let map = object(json)?;
        if map.get("type").and_then(Value::as_str) != Some(LIVE_STATE_TYPE) {
            return Err(LiveError::WrongType);
        }
        let state = map.get("state").and_then(Value::as_str).and_then(LiveStatus::from_str).ok_or(LiveError::UnknownState)?;
        let optional = |key: &'static str| -> Result<Option<String>, LiveError> {
            match map.get(key).and_then(Value::as_str) {
                Some(v) if v.len() > MAX_FIELD_BYTES => Err(LiveError::TooLong(key)),
                Some(v) => Ok(Some(v.to_string())),
                None => Ok(None),
            }
        };
        let mut peers = Vec::new();
        if let Some(list) = map.get("peers").and_then(Value::as_array) {
            if list.len() > MAX_PEERS {
                return Err(LiveError::TooLong("peers"));
            }
            for item in list {
                let id = item.get("id").and_then(Value::as_str).ok_or(LiveError::MissingField("id"))?;
                let display = item.get("display").and_then(Value::as_str).unwrap_or_default();
                if id.len() > MAX_FIELD_BYTES || display.len() > MAX_FIELD_BYTES {
                    return Err(LiveError::TooLong("peers"));
                }
                peers.push(Peer { id: id.to_string(), display: display.to_string() });
            }
        }
        Ok(Self { state, reason: optional("reason")?, saved_at: optional("savedAt")?, peers })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    const LIVE: &str = r#"{"type":"live","ticket":"tkt-SECRET-123","relayUrl":"wss://cloud.example/craft","room":"oc1:42:7","fileUrl":"/remote.php/dav/files/sam/Plan.docx","fileName":"Plan.docx","proto":"craft-live/1"}"#;

    #[test]
    fn live_message_round_trips() {
        let message = LiveMessage::parse(LIVE).expect("valid live message");
        assert_eq!(message.ticket(), "tkt-SECRET-123");
        assert_eq!(message.relay_url, "wss://cloud.example/craft");
        assert_eq!(message.room, "oc1:42:7");
        assert_eq!(message.file_url, "/remote.php/dav/files/sam/Plan.docx");
        assert_eq!(message.file_name, "Plan.docx");
        let again = LiveMessage::parse(&message.to_json()).expect("round trip");
        assert_eq!(again, message);
    }

    #[test]
    fn an_unknown_proto_is_a_typed_error() {
        let bad = LIVE.replace("craft-live/1", "craft-live/2");
        assert_eq!(LiveMessage::parse(&bad), Err(LiveError::UnsupportedProto("craft-live/2".into())));
        let none = LIVE.replace(r#","proto":"craft-live/1""#, "");
        assert_eq!(LiveMessage::parse(&none), Err(LiveError::MissingField("proto")));
    }

    #[test]
    fn hostile_or_incomplete_messages_are_errors_not_panics() {
        assert!(matches!(LiveMessage::parse("not json"), Err(LiveError::Malformed(_))));
        assert!(matches!(LiveMessage::parse("[]"), Err(LiveError::Malformed(_))));
        assert_eq!(LiveMessage::parse(&LIVE.replace(r#""type":"live""#, r#""type":"other""#)), Err(LiveError::WrongType));
        assert_eq!(LiveMessage::parse(&LIVE.replace("tkt-SECRET-123", "")), Err(LiveError::MissingField("ticket")));
        assert_eq!(LiveMessage::parse(&LIVE.replace("wss://cloud.example/craft", "https://x/craft")), Err(LiveError::BadRelayUrl));
        assert_eq!(LiveMessage::parse(&LIVE.replace("tkt-SECRET-123", &"a".repeat(MAX_TICKET_BYTES + 1))), Err(LiveError::TooLong("ticket")));
        let name = LiveMessage::parse(&LIVE.replace("Plan.docx\",\"proto", "../a/b.docx\",\"proto")).expect("name cleaned");
        assert_eq!(name.file_name, "b.docx");
    }

    #[test]
    fn debug_never_shows_the_ticket() {
        let message = LiveMessage::parse(LIVE).expect("valid");
        let shown = format!("{message:?} {:#?}", message);
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn live_state_round_trips() {
        let state = LiveState {
            state: LiveStatus::Reconnecting,
            reason: Some("relay closed".into()),
            saved_at: Some("2026-11-21T10:00:00Z".into()),
            peers: vec![Peer { id: "p1".into(), display: "Sam".into() }],
        };
        let json = state.to_json();
        assert!(json.contains(r#""type":"live-state""#) && json.contains(r#""state":"reconnecting""#) && json.contains(r#""savedAt""#));
        assert_eq!(LiveState::parse(&json), Ok(state));
        let bare = LiveState::parse(r#"{"type":"live-state","state":"live","peers":[]}"#).expect("optional fields");
        assert_eq!((bare.reason, bare.saved_at), (None, None));
        for s in ["connecting", "live", "readonly", "reconnecting", "refused"] {
            assert!(LiveState::parse(&format!(r#"{{"type":"live-state","state":"{s}","peers":[]}}"#)).is_ok(), "{s}");
        }
        assert!(LiveState::parse(r#"{"type":"live-state","state":"weird","peers":[]}"#).is_err());
    }

    #[test]
    fn ready_is_unprefixed() {
        assert_eq!(READY_JSON, r#"{"type":"ready"}"#);
    }
}
