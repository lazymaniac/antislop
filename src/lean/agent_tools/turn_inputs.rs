//! Exact prompt inputs live only in the current turn's tool handler.
use super::dependencies::page_text;
use crate::lean::prompt_context::{markdown, untrusted};
use crate::lean::tasks::{TaskContract, normalize_options_presentation};
use crate::{Result, invalid, require};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const INLINE_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum ReadFor {
    Plan,
    Work,
    Review,
    Reference,
    ScopeChange,
}

impl ReadFor {
    fn applies(self, tool: &str) -> bool {
        matches!(
            (self, tool),
            (Self::Plan, "submit_plan")
                | (Self::Work, "complete" | "select")
                | (Self::Review, "review")
                | (
                    Self::ScopeChange,
                    "submit_plan" | "revise_plan scope changes"
                )
        )
    }

    fn required(self) -> bool {
        !matches!(self, Self::Reference | Self::ScopeChange)
    }
}

#[derive(Default)]
pub(crate) struct TurnInputs {
    entries: BTreeMap<String, Entry>,
    inline_only: bool,
}

struct Entry {
    label: String,
    body: String,
    required: ReadFor,
    next: Option<String>,
    served: BTreeSet<Option<String>>,
    complete: bool,
}

impl TurnInputs {
    pub(crate) fn embedded() -> Self {
        Self {
            inline_only: true,
            ..Self::default()
        }
    }

    /// Preserve the original body and measure its UTF-8 bytes before formatting.
    pub(crate) fn block(&mut self, label: &str, body: &str, required: ReadFor) -> String {
        self.offload(label, body, required)
            .unwrap_or_else(|| untrusted(label, body))
    }

    pub(crate) fn offload(&mut self, label: &str, body: &str, required: ReadFor) -> Option<String> {
        if self.inline_only || body.len() <= INLINE_BYTES {
            return None;
        }
        Some(self.index_document(label, body, required))
    }

    /// Register exact material for selective reads even when it is small.
    pub(crate) fn reference(&mut self, label: &str, body: &str, required: ReadFor) -> String {
        if self.inline_only {
            untrusted(label, body)
        } else {
            self.index_document(label, body, required)
        }
    }

    fn index_document(&mut self, label: &str, body: &str, required: ReadFor) -> String {
        let reference = format!("input-{}", self.entries.len() + 1);
        self.entries.insert(
            reference.clone(),
            Entry {
                label: label.into(),
                body: body.into(),
                required,
                next: None,
                served: BTreeSet::new(),
                complete: false,
            },
        );
        format!(
            "{label} ({bytes} UTF-8 bytes): read_turn_input with input={reference}. Follow cursor until complete=true.",
            bytes = body.len(),
        )
    }

    pub(crate) fn task_contract<'a>(
        &mut self,
        task: &'a TaskContract,
        required: ReadFor,
    ) -> Cow<'a, TaskContract> {
        let mut contract = task.clone();
        normalize_options_presentation(&mut contract);
        let label = format!("Task contract for {}", contract.id);
        let Some(reference) = self.offload(&label, &markdown(&contract), required) else {
            return Cow::Borrowed(task);
        };
        contract.goal = match contract.summary.as_deref() {
            Some(summary) => format!("{}\n{reference}", crate::lean::tasks::bound(summary, 512)),
            None => reference,
        };
        contract.acceptance_criteria.clear();
        if !contract.outputs.is_empty() {
            contract.outputs = vec!["See the indexed task contract".into()];
        }
        Cow::Owned(contract)
    }

    pub(crate) fn read(&mut self, turn: &str, args: &Value) -> Result<Value> {
        let Some(requested) = args["input"].as_str() else {
            require(
                args["input"].is_null() && args["cursor"].is_null(),
                "input: supply an input ID, next, or omit it for the index",
            )?;
            return Ok(json!({"inputs": self.index()}));
        };
        let reference = if requested == "next" {
            require(
                args["cursor"].is_null(),
                "input=next advances its own cursor",
            )?;
            let Some((reference, _)) = self.entries.iter().find(|(_, entry)| entry.pending())
            else {
                return Ok(
                    json!({"complete":true,"all_required_complete":true,"inputs":self.index()}),
                );
            };
            reference.clone()
        } else {
            requested.to_owned()
        };
        let entry = self.entries.get_mut(&reference).ok_or_else(|| {
            invalid(format!(
                "input {reference:?} was not registered for this turn; omit input to list available IDs"
            ))
        })?;
        let cursor = match args.get("cursor") {
            None | Some(Value::Null) if requested == "next" => entry.next.clone(),
            None | Some(Value::Null) => None,
            Some(cursor) => Some(
                cursor
                    .as_str()
                    .ok_or_else(|| invalid("cursor: expected continuation cursor"))?
                    .to_owned(),
            ),
        };
        let replay = entry.served.contains(&cursor);
        if !replay && (entry.complete || cursor != entry.next) {
            let progress = match (entry.complete, entry.next.as_deref()) {
                (true, _) => "complete=true. Omit cursor to reread the first page.".into(),
                (false, Some(next)) => format!("complete=false. Continue with cursor={next}."),
                (false, None) => "complete=false. Omit cursor to read the first page.".into(),
            };
            return Err(invalid(format!("cursor for input={reference}: {progress}")));
        }
        let binding = crate::hash(format!("{turn}:{reference}").as_bytes());
        let (text, next) = page_text(&entry.body, &binding, &json!({"cursor":cursor}))?;
        if !replay {
            entry.served.insert(cursor);
            entry.next = next.clone();
            entry.complete = next.is_none();
        }
        let mut response = json!({"text":text,"complete":next.is_none(),"cursor":next});
        if requested == "next" {
            response["input"] = json!(reference);
            response["label"] = json!(entry.label);
            response["all_required_complete"] = json!(!self.entries.values().any(Entry::pending));
        }
        Ok(response)
    }

    fn index(&self) -> Vec<Value> {
        self.entries
            .iter()
            .map(|(id, entry)| {
                json!({"input":id, "label":entry.label, "bytes":entry.body.len(),
                "required":entry.required.required(),
                "required_for_scope_change":matches!(entry.required, ReadFor::ScopeChange),
                "complete":entry.complete,"next_cursor":entry.next})
            })
            .collect()
    }

    pub(crate) fn required_read_instructions(&self) -> String {
        if !self.entries.values().any(Entry::pending) {
            return String::new();
        }
        let required: Vec<_> = self
            .index()
            .into_iter()
            .filter(|entry| entry["required"] == true)
            .collect();
        format!(
            "Before starting the assignment, call read_turn_input with input=next until all_required_complete=true. Omit input to see progress.\n{}",
            untrusted("Required turn inputs", &markdown(&required))
        )
    }

    pub(crate) fn require_reads(&self, tool: &str) -> Result<()> {
        let missing: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.required.applies(tool) && !entry.complete)
            .map(|(reference, _)| reference.as_str())
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(invalid(format!(
            "Before {tool}, use read_turn_input to read every page of: {}",
            missing.join(", ")
        )))
    }
}

impl Entry {
    fn pending(&self) -> bool {
        !self.complete && self.required.required()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_next_show_and_advance_the_required_cursor() {
        let mut inputs = TurnInputs::default();
        let body = "ś".repeat(10_000);
        inputs.offload("Optional reference", &body, ReadFor::Reference);
        inputs.offload("Required task contract", &body, ReadFor::Review);
        inputs.offload("Prior review", &body, ReadFor::Review);
        let index = inputs.read("turn", &json!({})).unwrap();
        assert_eq!(index["inputs"][1]["label"], "Required task contract");
        let first = inputs.read("turn", &json!({"input":"next"})).unwrap();
        assert_eq!(first["input"], "input-2");
        assert!(!first["cursor"].is_null());
        let index = inputs.read("turn", &json!({})).unwrap();
        assert_eq!(index["inputs"][1]["next_cursor"], first["cursor"]);
        let wrong = json!({"input":"input-2","cursor":"guess"});
        let error = inputs.read("turn", &wrong).unwrap_err().to_string();
        assert!(error.contains(first["cursor"].as_str().unwrap()), "{error}");
        let last = inputs.read("turn", &json!({"input":"next"})).unwrap();
        assert_eq!(last["complete"], true);
        assert_eq!(last["all_required_complete"], false);
        let error = inputs.read("turn", &wrong).unwrap_err().to_string();
        assert!(error.contains("complete=true"), "{error}");
        let second = inputs.read("turn", &json!({"input":"next"})).unwrap();
        assert_eq!(second["input"], "input-3");
        let last = inputs.read("turn", &json!({"input":"next"})).unwrap();
        assert_eq!(last["all_required_complete"], true);
        inputs.require_reads("review").unwrap();
        assert_eq!(
            inputs.read("turn", &json!({"input":"next"})).unwrap()["complete"],
            true
        );
    }

    #[test]
    fn threshold_order_replay_and_turn_binding_use_utf8_bytes() {
        let mut inputs = TurnInputs::default();
        let edge = "é".repeat(INLINE_BYTES / 2);
        assert!(inputs.offload("Scope", &edge, ReadFor::Plan).is_none());
        assert!(
            inputs
                .offload("Scope", &(edge + "a"), ReadFor::Plan)
                .is_some()
        );

        let body = "a👩‍💻é".repeat(3000);
        for label in ["Contract", "Earlier contract"] {
            assert!(inputs.offload(label, &body, ReadFor::Review).is_some());
        }
        assert!(inputs.require_reads("submit_plan").is_err());
        assert!(inputs.require_reads("review").is_err());
        for tool in [
            "report_blocker",
            "ask",
            "wait_for_answer",
            "complete",
            "select",
        ] {
            inputs.require_reads(tool).unwrap();
        }

        let start = json!({"input":"input-2"});
        let first = inputs.read("active", &start).unwrap();
        assert_eq!(first, inputs.read("active", &start).unwrap());
        let next = json!({"input":"input-2", "cursor": first["cursor"]});
        assert!(inputs.read("stale", &next).is_err());
        assert!(
            inputs
                .read(
                    "active",
                    &json!({"input":"input-3","cursor":first["cursor"]})
                )
                .is_err()
        );
        let binding = crate::hash("active:input-2".as_bytes());
        assert!(
            inputs
                .read(
                    "active",
                    &json!({"input":"input-2", "cursor":format!("{binding}:{}", body.len())})
                )
                .is_err()
        );

        let mut actual = first["text"].as_str().unwrap().to_owned();
        let mut page = inputs.read("active", &next).unwrap();
        loop {
            actual.push_str(page["text"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            page = inputs
                .read(
                    "active",
                    &json!({"input":"input-2","cursor":page["cursor"]}),
                )
                .unwrap();
        }
        assert_eq!(actual, body);
        assert_eq!(first, inputs.read("active", &start).unwrap());
        let missing = inputs.require_reads("review").unwrap_err().to_string();
        assert!(!missing.contains("input-2"));
        assert!(missing.contains("input-3"));
    }
}
