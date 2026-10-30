mod all;
use super::super::{Request, WireRequest};

pub(super) fn decode_session(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    all::decode_group_all(wire)
}
