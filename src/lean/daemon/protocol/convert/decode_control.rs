mod all;
use super::super::{Request, WireRequest};

pub(super) fn decode_control(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    all::decode_group_all(wire)
}
