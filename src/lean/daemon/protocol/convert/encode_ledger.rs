mod reads;
mod writes;
use super::super::{Request, WireRequest};

pub(super) fn encode_ledger(request: Request) -> WireRequest {
    reads::encode_group_reads(request)
        .or_else(|request| writes::encode_group_writes(*request))
        .unwrap_or_else(|_| unreachable!("request routed to wrong protocol group"))
}
