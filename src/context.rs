//! Immutable, turn-scoped context. Reading a page grants no filesystem access.
use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub mod bridge;
mod sources;
pub mod stdio;
pub mod transport;
pub const FORMAT: &str = "pinned-context-v1";
pub const MAX_PAGE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub run_id: String,
    pub spec_revision: u64,
    pub turn_id: String,
    pub input_hash: String,
    pub prompt_hash: String,
    pub schema_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub source_refs: BTreeSet<String>,
    pub utf8_bytes: usize,
    pub utf8: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub scope: Scope,
    pub documents: BTreeMap<String, Document>,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Bundle {
    pub fn new(scope: Scope, sources: impl IntoIterator<Item = (String, String)>) -> Result<Self> {
        let mut documents = BTreeMap::<String, Document>::new();
        let mut seen = BTreeSet::new();
        for (source, body) in sources {
            require(
                !source.trim().is_empty() && seen.insert(source.clone()),
                "ambiguous context source reference",
            )?;
            documents
                .entry(hash(&body))
                .or_insert_with(|| Document {
                    source_refs: BTreeSet::new(),
                    utf8_bytes: body.len(),
                    utf8: body,
                })
                .source_refs
                .insert(source);
        }
        let bundle = Self {
            format: FORMAT.into(),
            scope,
            documents,
        };
        bundle.validate()?;
        Ok(bundle)
    }
    pub fn for_turn(request: &crate::providers::TurnRequest<'_>) -> Result<Self> {
        let mut sources = request.run.sources();
        // Runtime source references already distinguish file/requirement/plan
        // namespaces. Register the original captured bytes, never live reads.
        require(
            !sources.contains_key("prepared-prompt"),
            "reserved prepared context reference",
        )?;
        sources.insert("prepared-prompt".into(), request.prompt.into());
        sources::extend(request.run, request.prompt, &mut sources)?;
        Self::new(
            Scope {
                run_id: request.run.id.clone(),
                spec_revision: request.run.spec_revision,
                turn_id: request.turn_id.into(),
                input_hash: hash(serde_json::to_vec(&request.run.input_snapshot)?),
                prompt_hash: hash(request.prompt),
                schema_hash: hash(serde_json::to_vec(request.schema)?),
            },
            sources,
        )
    }
    pub fn validate(&self) -> Result<()> {
        require(
            self.format == FORMAT
                && !self.scope.run_id.trim().is_empty()
                && !self.scope.turn_id.trim().is_empty()
                && self.scope.spec_revision > 0
                && [
                    &self.scope.input_hash,
                    &self.scope.prompt_hash,
                    &self.scope.schema_hash,
                ]
                .iter()
                .all(|v| digest(v))
                && !self.documents.is_empty(),
            "invalid pinned context scope",
        )?;
        let mut seen = BTreeSet::new();
        for (key, document) in &self.documents {
            require(
                digest(key)
                    && hash(&document.utf8) == *key
                    && document.utf8_bytes == document.utf8.len()
                    && !document.source_refs.is_empty()
                    && document
                        .source_refs
                        .iter()
                        .all(|s| !s.trim().is_empty() && seen.insert(s)),
                "invalid pinned context document",
            )?;
        }
        Ok(())
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub bundle_hash: String,
    pub document_hash: String,
    #[serde(deserialize_with = "byte_count")]
    pub offset: usize,
    #[serde(deserialize_with = "byte_count")]
    pub max_bytes: usize,
}
// JSON Schema integers include 512.0. MCP clients using protobuf Structs may
// encode integral byte counts this way. Never accept rounded/fractional counts.
fn byte_count<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<usize, D::Error> {
    let value = serde_json::Value::deserialize(decoder)?;
    if let Some(integer) = value.as_u64() {
        return usize::try_from(integer)
            .map_err(|_| serde::de::Error::custom("byte count overflow"));
    }
    if let Some(number) = value.as_f64()
        && (0.0..=9_007_199_254_740_991.0).contains(&number)
        && number.fract() == 0.0
    {
        return usize::try_from(number as u64)
            .map_err(|_| serde::de::Error::custom("byte count overflow"));
    }
    Err(serde::de::Error::custom(
        "byte count must be an exact nonnegative integer",
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub sequence: u64,
    pub bundle_hash: String,
    pub scope: Scope,
    pub document_hash: String,
    pub total_bytes: usize,
    pub offset: usize,
    pub end: usize,
    pub chunk_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub receipt: Receipt,
    pub next_offset: Option<usize>,
    pub content: String,
}

#[derive(Clone)]
pub struct Reader {
    bundle: Bundle,
    bundle_hash: String,
    index_hash: String,
    index: String,
}
impl Reader {
    pub fn bundle_hash(&self) -> &str {
        &self.bundle_hash
    }
    pub fn index_hash(&self) -> &str {
        &self.index_hash
    }
    pub fn from_bytes(bytes: &[u8], expected_hash: &str) -> Result<Self> {
        require(
            digest(expected_hash) && hash(bytes) == expected_hash,
            "pinned context bundle changed",
        )?;
        let bundle: Bundle = crate::transport::decode(bytes)
            .map_err(|_| crate::Error::Invalid("invalid pinned context encoding".into()))?;
        bundle.validate()?;
        let index = serde_json::to_string(
            &serde_json::json!({"format":FORMAT,"bundle_hash":expected_hash,"scope":bundle.scope,
            "documents":bundle.documents.iter().map(|(id,d)|serde_json::json!({"document_hash":id,"utf8_bytes":d.utf8_bytes,"source_refs":d.source_refs})).collect::<Vec<_>>() }),
        )?;
        Ok(Self {
            bundle,
            bundle_hash: expected_hash.into(),
            index_hash: hash(&index),
            index,
        })
    }
    pub fn index_bytes(&self) -> usize {
        self.index.len()
    }
    pub fn read(&self, request: &Read, sequence: u64) -> Result<Page> {
        require(sequence > 0, "context receipt sequence starts at one")?;
        require(
            request.bundle_hash == self.bundle_hash,
            "context read belongs to another bundle",
        )?;
        require(
            (4..=MAX_PAGE_BYTES).contains(&request.max_bytes),
            "context page size must be between 4 and 32768 bytes",
        )?;
        let text = if request.document_hash == self.index_hash {
            &self.index
        } else {
            &self
                .bundle
                .documents
                .get(&request.document_hash)
                .ok_or_else(|| {
                    crate::Error::Invalid("context document is outside the pinned bundle".into())
                })?
                .utf8
        };
        require(
            request.offset <= text.len() && text.is_char_boundary(request.offset),
            "invalid context byte offset",
        )?;
        let mut end = request
            .offset
            // Leave room for JSON escaping and receipt metadata in native tool
            // output. A requested maximum is an upper bound, not a fixed size.
            .saturating_add(request.max_bytes.min(16 * 1024))
            .min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let content = text[request.offset..end].to_owned();
        Ok(Page {
            receipt: Receipt {
                sequence,
                bundle_hash: self.bundle_hash.clone(),
                scope: self.bundle.scope.clone(),
                document_hash: request.document_hash.clone(),
                total_bytes: text.len(),
                offset: request.offset,
                end,
                chunk_hash: hash(&content),
            },
            next_offset: (end < text.len()).then_some(end),
            content,
        })
    }
    pub fn authenticate(&self, page: &Page) -> Result<()> {
        let size = page
            .receipt
            .end
            .checked_sub(page.receipt.offset)
            .ok_or_else(|| crate::Error::Invalid("invalid context receipt range".into()))?;
        let expected = self.read(
            &Read {
                bundle_hash: page.receipt.bundle_hash.clone(),
                document_hash: page.receipt.document_hash.clone(),
                offset: page.receipt.offset,
                max_bytes: size.max(4),
            },
            page.receipt.sequence,
        )?;
        require(
            expected.receipt == page.receipt
                && expected.content == page.content
                && expected.next_offset == page.next_offset,
            "context page differs from its pinned read",
        )
    }
}
