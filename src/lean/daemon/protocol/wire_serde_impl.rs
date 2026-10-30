use super::wire::WireRequest;
use crate::lean::daemon::Request;
use serde::{Deserialize, Serialize};

impl Serialize for Request {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        super::convert::encode(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WireRequest::deserialize(deserializer).map(super::convert::decode)
    }
}
