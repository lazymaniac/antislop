mod reads;
mod writes;
use super::super::{Request, WireRequest};

pub(super) fn decode_ledger(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    reads::decode_group_reads(wire).or_else(|wire| writes::decode_group_writes(*wire))
}
