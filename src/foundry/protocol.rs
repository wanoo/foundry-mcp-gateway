//! Trames Engine.IO / Socket.IO de Foundry VTT (protocole vérifié sur v13).
//!   "0{...}"      handshake Engine.IO
//!   "2" / "3"     ping serveur / pong client (sinon coupure ~10 min)
//!   "40"          connect Socket.IO (client) · "40{...}" ack serveur
//!   "42[ev,...]"  événement broadcast (sans ack)
//!   "42N[ev,...]" événement émis avec ack id N
//!   "43N[...]"    réponse d'ack à l'id N

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Handshake(Value),
    Ping,
    SocketConnected(Value),
    /// Broadcast serveur : (événement, arguments)
    Event(String, Vec<Value>),
    /// Réponse d'ack : (ack_id, payload)
    Ack(u64, Vec<Value>),
    Other(String),
}

pub fn parse_frame(msg: &str) -> Frame {
    if msg == "2" {
        return Frame::Ping;
    }
    if let Some(rest) = msg.strip_prefix("0")
        && rest.starts_with('{')
        && let Ok(v) = serde_json::from_str(rest)
    {
        return Frame::Handshake(v);
    }
    if let Some(rest) = msg.strip_prefix("40") {
        let v = serde_json::from_str(rest).unwrap_or(Value::Null);
        return Frame::SocketConnected(v);
    }
    if let Some(rest) = msg.strip_prefix("43") {
        // "43<digits>[...]"
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = digits.parse::<u64>()
            && let Ok(Value::Array(payload)) = serde_json::from_str(&rest[digits.len()..])
        {
            return Frame::Ack(id, payload);
        }
    }
    if let Some(rest) = msg.strip_prefix("42") {
        // broadcast pur : "42[...]" (un ack id ferait "42<digits>[")
        if rest.starts_with('[')
            && let Ok(Value::Array(mut arr)) = serde_json::from_str::<Value>(rest).map(|v| {
                if let Value::Array(a) = v {
                    Value::Array(a)
                } else {
                    Value::Null
                }
            })
            && let Some(Value::String(event)) = arr.first().cloned()
        {
            arr.remove(0);
            return Frame::Event(event, arr);
        }
    }
    Frame::Other(msg.to_string())
}

pub const PONG: &str = "3";
pub const SOCKET_CONNECT: &str = "40";

/// Émission d'un événement, avec ou sans ack id : `42[..]` / `42N[..]`.
pub fn build_emit(event: &str, args: &[Value], ack_id: Option<u64>) -> String {
    let mut arr = vec![Value::String(event.to_string())];
    arr.extend(args.iter().cloned());
    let json = serde_json::to_string(&Value::Array(arr)).unwrap_or_default();
    match ack_id {
        Some(id) => format!("42{id}{json}"),
        None => format!("42{json}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_les_trames_de_base() {
        assert_eq!(parse_frame("2"), Frame::Ping);
        assert!(matches!(
            parse_frame("0{\"sid\":\"x\",\"pingInterval\":20000}"),
            Frame::Handshake(_)
        ));
        assert!(matches!(
            parse_frame("40{\"sid\":\"y\"}"),
            Frame::SocketConnected(_)
        ));
    }

    #[test]
    fn broadcast_vs_ack() {
        match parse_frame(r#"42["session",{"userId":"u1"}]"#) {
            Frame::Event(ev, args) => {
                assert_eq!(ev, "session");
                assert_eq!(args[0]["userId"], "u1");
            }
            f => panic!("attendu Event, reçu {f:?}"),
        }
        match parse_frame(r#"4312[{"action":"get","result":[]}]"#) {
            Frame::Ack(id, payload) => {
                assert_eq!(id, 12);
                assert_eq!(payload[0]["action"], "get");
            }
            f => panic!("attendu Ack, reçu {f:?}"),
        }
        // une émission AVEC ack id n'est PAS un broadcast
        assert!(matches!(
            parse_frame(r#"4212["modifyDocument",{}]"#),
            Frame::Other(_)
        ));
    }

    #[test]
    fn build_emit_formats() {
        assert_eq!(build_emit("world", &[], Some(0)), r#"420["world"]"#);
        assert_eq!(
            build_emit("pause", &[json!(true)], None),
            r#"42["pause",true]"#
        );
    }
}
