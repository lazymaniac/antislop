mod decode_control;
mod decode_conversation;
mod decode_history;
mod decode_ledger;
mod decode_session;
mod decode_terminal;
mod encode_control;
mod encode_conversation;
mod encode_history;
mod encode_ledger;
mod encode_session;
mod encode_terminal;
use super::{Request, WireRequest};

pub(super) fn encode(request: Request) -> WireRequest {
    match request {
        request @ Request::Terminal(..) => encode_terminal::encode_terminal(request),
        request @ Request::Session(..) => encode_session::encode_session(request),
        request @ Request::History(..) => encode_history::encode_history(request),
        request @ Request::Ledger(..) => encode_ledger::encode_ledger(request),
        request @ Request::Conversation(..) => encode_conversation::encode_conversation(request),
        request @ Request::Control(..) => encode_control::encode_control(request),
    }
}

pub(super) fn decode(wire: WireRequest) -> Request {
    let wire = match decode_terminal::decode_terminal(wire) {
        Ok(request) => return request,
        Err(wire) => *wire,
    };
    let wire = match decode_session::decode_session(wire) {
        Ok(request) => return request,
        Err(wire) => *wire,
    };
    let wire = match decode_history::decode_history(wire) {
        Ok(request) => return request,
        Err(wire) => *wire,
    };
    let wire = match decode_ledger::decode_ledger(wire) {
        Ok(request) => return request,
        Err(wire) => *wire,
    };
    let wire = match decode_conversation::decode_conversation(wire) {
        Ok(request) => return request,
        Err(wire) => *wire,
    };
    decode_control::decode_control(wire)
        .unwrap_or_else(|_| unreachable!("wire request belongs to a known protocol group"))
}
