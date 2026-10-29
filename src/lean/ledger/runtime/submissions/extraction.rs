use super::*;
use serde::Deserialize;
/// A requirement as the extraction analyst submits it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedEntry {
    id: String,
    /// The user's words the requirement comes from, copied exactly.
    quote: String,
    /// The source the quote is in, when it is not the first that has it.
    #[serde(default)]
    source: Option<String>,
    interpretation: String,
    #[serde(default)]
    qualifiers: Vec<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default = "yes")]
    mandatory: bool,
    #[serde(default)]
    criteria: Vec<Criterion>,
    #[serde(default)]
    task_ids: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Core {
    /// The extraction `submit_ledger` makes for the conversation's current
    /// run, checked as the runtime will apply it.
    pub(crate) fn submitted_extraction(
        &self,
        conversation: &str,
        args: &serde_json::Value,
    ) -> Result<Extraction> {
        let run = self
            .store
            .planning_run(conversation)?
            .ok_or_else(|| Error::Invalid("there is no requirements run to extract".into()))?;
        let submitted: Vec<SubmittedEntry> = serde_json::from_value(args["entries"].clone())
            .map_err(|error| Error::Invalid(format!("entries: {error}")))?;
        let sources = self.source_texts(conversation, &run.id)?;
        let existing = self.store.ledger_entries(conversation, &run.id, None)?;
        let entries = extraction_entries(submitted, &sources, &existing)?;
        let extraction = Extraction {
            run_id: run.id.clone(),
            expected_spec_revision: run.spec_revision,
            entries,
        };
        self.validate_extraction(conversation, &run, &existing, extraction.clone())?;
        Ok(extraction)
    }

    fn source_texts(&self, conversation: &str, run_id: &str) -> Result<Vec<(String, String)>> {
        self.store
            .ledger_sources(conversation, run_id)?
            .into_iter()
            .map(|source| {
                let text =
                    self.store
                        .ledger_source_text(conversation, run_id, &source.id, &self.state)?;
                Ok((source.id, text))
            })
            .collect()
    }
}

fn extraction_entries(
    submitted: Vec<SubmittedEntry>,
    sources: &[(String, String)],
    existing: &[LedgerEntry],
) -> Result<Vec<EntrySpec>> {
    submitted
        .into_iter()
        .enumerate()
        .map(|(index, entry)| extraction_entry(entry, index, sources, existing))
        .collect()
}

fn extraction_entry(
    entry: SubmittedEntry,
    index: usize,
    sources: &[(String, String)],
    existing: &[LedgerEntry],
) -> Result<EntrySpec> {
    let position = existing
        .iter()
        .find(|old| old.spec.id == entry.id)
        .map_or((existing.len() + index) as u32, |old| old.spec.position);
    Ok(EntrySpec {
        source: quoted_span(sources, &entry.id, &entry.quote, entry.source.as_deref())?,
        id: entry.id,
        interpretation: entry.interpretation,
        qualifiers: entry.qualifiers,
        parent: entry.parent,
        depends_on: entry.depends_on,
        mandatory: entry.mandatory,
        implementation_owners: Vec::new(),
        verification_owners: Vec::new(),
        criteria: entry.criteria,
        task_ids: entry.task_ids,
        position,
        disposition: Disposition::Active,
    })
}

/// Where in the sources `quote` stands: in the named source, or else the
/// first that has it (a later message may repeat the user's words, as an
/// approval does); there it must occur once, so the range is unambiguous.
pub(super) fn quoted_span(
    sources: &[(String, String)],
    entry: &str,
    quote: &str,
    source: Option<&str>,
) -> Result<SourceSpan> {
    let quote = quote.trim();
    require(
        !quote.is_empty(),
        &format!("{entry}: quote the user's words the requirement comes from"),
    )?;
    let (id, text, start) = locate_quote(sources, entry, quote, source)?;
    let span = SourceSpan {
        source_id: id.clone(),
        start,
        end: start + quote.len(),
    };
    span.text(text).map_err(|_| {
        Error::Invalid(format!(
            "{entry}: the quote cuts through a character; quote whole words"
        ))
    })?;
    Ok(span)
}

fn locate_quote<'a>(
    sources: &'a [(String, String)],
    entry: &str,
    quote: &str,
    source: Option<&str>,
) -> Result<(&'a String, &'a String, usize)> {
    validate_source_name(sources, entry, source)?;
    let named = |id: &String| source.is_none_or(|source| source == id);
    let (id, text) = sources
        .iter()
        .find(|(id, text)| named(id) && text.contains(quote))
        .ok_or_else(|| {
            Error::Invalid(format!(
                "{entry}: the quote does not occur in {}; copy the user's words exactly",
                source.unwrap_or("any source")
            ))
        })?;
    let start = unique_quote_start(text, quote, entry, id)?;
    Ok((id, text, start))
}

fn validate_source_name(
    sources: &[(String, String)],
    entry: &str,
    source: Option<&str>,
) -> Result<()> {
    if let Some(source) = source {
        require(
            sources.iter().any(|(id, _)| id == source),
            &format!(
                "{entry}: source {source} is not one of {}",
                sources
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )?;
    }
    Ok(())
}

fn unique_quote_start(text: &str, quote: &str, entry: &str, source: &str) -> Result<usize> {
    // Count overlapping occurrences too: `aaa` occurs twice in `aaaa`.
    let found: Vec<usize> = text
        .char_indices()
        .map(|(start, _)| start)
        .filter(|&start| text[start..].starts_with(quote))
        .collect();
    require(
        found.len() == 1,
        &format!(
            "{entry}: the quote occurs {} times in source {source}; quote a longer passage so it names one place",
            found.len()
        ),
    )?;
    Ok(found[0])
}
