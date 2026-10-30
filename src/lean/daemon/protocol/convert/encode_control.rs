mod all;
use super::super::{Request, WireRequest};

pub(super) fn encode_control(request: Request) -> WireRequest {
    all::encode_group_all(request)
        .unwrap_or_else(|_| unreachable!("request routed to wrong protocol group"))
}
