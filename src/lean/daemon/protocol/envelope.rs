use super::Request;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestEnvelope {
    pub(crate) protocol: u32,
    pub(crate) request: Request,
}
