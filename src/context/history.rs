//! Lossless historical turns with a flat content-addressed source manifest.
//! Native input/receipt bytes remain original; historical delivery is never
//! promoted to delivery in the current turn or to semantic approval.
use super::*;
use crate::sessions::TurnRecord;

const FORMAT: &str = "turn-archive-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub state: crate::sessions::TurnState,
    pub provider_completed: bool,
    pub failure: Option<crate::providers::FailureKind>,
    pub rejected_output_hash: Option<String>,
    pub observed_required_bytes: Option<usize>,
    pub required_bytes: Option<usize>,
}
impl Status {
    fn capture(turn: &TurnRecord) -> Result<Self> {
        let coverage = turn
            .context
            .as_ref()
            .map(transport::Transport::progress)
            .transpose()?;
        Ok(Self {
            state: turn.state,
            provider_completed: turn.provider_completed,
            failure: turn.failure,
            rejected_output_hash: turn.rejected_output.as_ref().map(|r| r.hash.clone()),
            observed_required_bytes: coverage.map(|v| v.0),
            required_bytes: coverage.map(|v| v.1),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub turn_id: String,
    pub session_id: String,
    pub source_ref: String,
    pub document_hash: String,
    pub utf8_bytes: usize,
    pub original_turn_hash: String,
    pub original_turn_bytes: usize,
    pub status: Status,
}
impl Reference {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.turn_id.is_empty()
                && !self.session_id.is_empty()
                && self.source_ref == format!("task-history-turn:{}", self.turn_id)
                && digest(&self.document_hash)
                && digest(&self.original_turn_hash)
                && self.utf8_bytes > 0
                && self.original_turn_bytes > 0
                && self
                    .status
                    .rejected_output_hash
                    .as_deref()
                    .is_none_or(digest)
                && match (
                    self.status.observed_required_bytes,
                    self.status.required_bytes,
                ) {
                    (None, None) => true,
                    (Some(observed), Some(required)) => required > 0 && observed <= required,
                    _ => false,
                },
            "invalid historical task turn reference",
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentManifest {
    source_refs: BTreeSet<String>,
    utf8_bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchivedContext {
    version: String,
    scope: Scope,
    bundle_format: String,
    documents: BTreeMap<String, DocumentManifest>,
    native_prompt_hash: String,
    receipts: Vec<Receipt>,
    observed: BTreeSet<u64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    format: String,
    turn: TurnRecord,
    context: Option<ArchivedContext>,
    original_turn_hash: String,
    original_turn_bytes: usize,
}
impl Archive {
    fn capture(turn: &TurnRecord) -> Result<Self> {
        turn.validate()?;
        let original = serde_json::to_vec(turn)?;
        let mut metadata = turn.clone();
        metadata.context = None;
        let context = turn.context.as_ref().map(|c| ArchivedContext {
            version: c.version.clone(),
            scope: c.bundle.scope.clone(),
            bundle_format: c.bundle.format.clone(),
            documents: c
                .bundle
                .documents
                .iter()
                .map(|(hash, d)| {
                    (
                        hash.clone(),
                        DocumentManifest {
                            source_refs: d.source_refs.clone(),
                            utf8_bytes: d.utf8_bytes,
                        },
                    )
                })
                .collect(),
            native_prompt_hash: c.native_prompt_hash.clone(),
            receipts: c.receipts.clone(),
            observed: c.observed.clone(),
        });
        Ok(Self {
            format: FORMAT.into(),
            turn: metadata,
            context,
            original_turn_hash: hash(&original),
            original_turn_bytes: original.len(),
        })
    }
    fn reference(&self, status: Status) -> Result<Reference> {
        let body = serde_json::to_string(self)?;
        Ok(Reference {
            turn_id: self.turn.id.clone(),
            session_id: self.turn.session_id.clone(),
            source_ref: format!("task-history-turn:{}", self.turn.id),
            document_hash: hash(&body),
            utf8_bytes: body.len(),
            original_turn_hash: self.original_turn_hash.clone(),
            original_turn_bytes: self.original_turn_bytes,
            status,
        })
    }
}

pub fn reference(turn: &TurnRecord) -> Result<Reference> {
    Archive::capture(turn)?.reference(Status::capture(turn)?)
}

fn insert(sources: &mut BTreeMap<String, String>, reference: String, body: String) -> Result<()> {
    if let Some(existing) = sources.get(&reference) {
        require(
            existing == &body,
            "conflicting historical context source alias",
        )
    } else {
        sources.insert(reference, body);
        Ok(())
    }
}

/// Register an authenticated original and its flat source bodies. Aliases for
/// content depend only on its hash, so repeated replacements do not recursively
/// wrap earlier aliases or copy nested context bundles into the archive body.
pub fn extend(
    turn: &TurnRecord,
    expected: &Reference,
    sources: &mut BTreeMap<String, String>,
) -> Result<()> {
    expected.validate()?;
    let archive = Archive::capture(turn)?;
    require(
        archive.reference(Status::capture(turn)?)? == *expected,
        "historical task turn differs from its approved reference",
    )?;
    let mut additions = BTreeMap::new();
    insert(
        &mut additions,
        expected.source_ref.clone(),
        serde_json::to_string(&archive)?,
    )?;
    if let Some(context) = &turn.context {
        for (id, d) in &context.bundle.documents {
            insert(
                &mut additions,
                format!("task-history-content:{id}"),
                d.utf8.clone(),
            )?;
        }
    }
    // Do not leave a partial source registration on a conflicting alias.
    require(
        additions
            .iter()
            .all(|(r, b)| sources.get(r).is_none_or(|old| old == b)),
        "conflicting historical context source alias",
    )?;
    sources.extend(additions);
    Ok(())
}

/// Reconstruct the exact original record from addressed bytes, useful for
/// inspection/export. This does not acknowledge historical native reads.
pub fn restore(
    reference: &Reference,
    documents: &BTreeMap<String, Document>,
) -> Result<TurnRecord> {
    reference.validate()?;
    let body = documents
        .get(&reference.document_hash)
        .ok_or_else(|| crate::Error::Invalid("historical task archive unavailable".into()))?;
    require(
        body.utf8_bytes == reference.utf8_bytes
            && body.utf8.len() == reference.utf8_bytes
            && hash(&body.utf8) == reference.document_hash
            && body.source_refs.contains(&reference.source_ref),
        "historical task archive identity changed",
    )?;
    let archive: Archive = crate::transport::decode(body.utf8.as_bytes())?;
    require(
        archive.format == FORMAT
            && archive.turn.id == reference.turn_id
            && archive.turn.session_id == reference.session_id
            && archive.turn.context.is_none()
            && archive.original_turn_hash == reference.original_turn_hash
            && archive.original_turn_bytes == reference.original_turn_bytes,
        "historical task archive metadata changed",
    )?;
    let mut turn = archive.turn;
    if let Some(context) = archive.context {
        let mut originals = BTreeMap::new();
        for (id, manifest) in context.documents {
            let source = documents.get(&id).ok_or_else(|| {
                crate::Error::Invalid("historical context document unavailable".into())
            })?;
            require(
                hash(&source.utf8) == id
                    && source.utf8.len() == manifest.utf8_bytes
                    && source.utf8_bytes == manifest.utf8_bytes,
                "historical context document bytes changed",
            )?;
            originals.insert(
                id,
                Document {
                    source_refs: manifest.source_refs,
                    utf8_bytes: manifest.utf8_bytes,
                    utf8: source.utf8.clone(),
                },
            );
        }
        let bundle = Bundle {
            format: context.bundle_format,
            scope: context.scope,
            documents: originals,
        };
        bundle.validate()?;
        let restored: transport::Transport = serde_json::from_value(
            serde_json::json!({"version":context.version,
            "bundle":bundle,"native_prompt_hash":context.native_prompt_hash,"receipts":context.receipts,"observed":context.observed}),
        )?;
        restored.validate()?;
        turn.context = Some(restored);
    }
    turn.validate()?;
    let bytes = serde_json::to_vec(&turn)?;
    require(
        hash(&bytes) == reference.original_turn_hash
            && bytes.len() == reference.original_turn_bytes,
        "historical task turn reconstruction changed original bytes",
    )?;
    require(
        reference.status == Status::capture(&turn)?,
        "historical task turn status projection changed",
    )?;
    Ok(turn)
}
