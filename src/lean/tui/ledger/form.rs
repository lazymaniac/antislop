use crate::lean::ledger::*;
use crate::lean::tui::InputBuffer;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum FormField {
    Id,
    Interpretation,
    Qualifiers,
    Criteria,
    Parent,
    Dependencies,
    ImplementationOwners,
    VerificationOwners,
    Tasks,
    Disposition,
    Reason,
    Source,
    Mandatory,
    Position,
}
impl FormField {
    pub const ALL: [Self; 14] = [
        Self::Id,
        Self::Interpretation,
        Self::Qualifiers,
        Self::Criteria,
        Self::Parent,
        Self::Dependencies,
        Self::ImplementationOwners,
        Self::VerificationOwners,
        Self::Tasks,
        Self::Disposition,
        Self::Reason,
        Self::Source,
        Self::Mandatory,
        Self::Position,
    ];
    pub(super) fn next(self, backwards: bool) -> Self {
        Self::ALL
            [(self as usize + if backwards { Self::ALL.len() - 1 } else { 1 }) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub base: EntrySpec,
    pub revision: i64,
    pub fields: Vec<InputBuffer>,
    pub field: FormField,
    pub operation_id: String,
}
impl Form {
    pub fn new(spec: EntrySpec, revision: i64) -> Self {
        let fields = form_values(&spec)
            .into_iter()
            .map(|text| InputBuffer::with_text(crate::lean::tui::sanitize_paste(&text)))
            .collect();
        Self {
            base: spec,
            revision,
            fields,
            field: FormField::Interpretation,
            operation_id: crate::id(),
        }
    }
    pub fn spec(&self) -> crate::Result<EntrySpec> {
        let mut spec = self.base.clone();
        self.update_identity_and_text(&mut spec)?;
        self.update_links(&mut spec);
        self.update_policy(&mut spec)?;
        Ok(spec)
    }

    fn update_identity_and_text(&self, spec: &mut EntrySpec) -> crate::Result<()> {
        spec.id = self.field_text(FormField::Id);
        crate::require(
            self.base.source.source_id == "new" || spec.id == self.base.id,
            "existing record identity is immutable",
        )?;
        spec.interpretation = self.field_text(FormField::Interpretation);
        spec.qualifiers = self
            .field_text(FormField::Qualifiers)
            .lines()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        spec.criteria = parse_criteria(&self.field_text(FormField::Criteria))?;
        spec.parent = if self.field_text(FormField::Parent).is_empty() {
            None
        } else {
            Some(self.field_text(FormField::Parent))
        };
        Ok(())
    }

    fn update_links(&self, spec: &mut EntrySpec) {
        spec.depends_on = self.field_ids(FormField::Dependencies);
        spec.implementation_owners = self.field_ids(FormField::ImplementationOwners);
        spec.verification_owners = self.field_ids(FormField::VerificationOwners);
        spec.task_ids = self.field_ids(FormField::Tasks);
    }

    fn update_policy(&self, spec: &mut EntrySpec) -> crate::Result<()> {
        spec.disposition =
            serde_json::from_value(Value::String(self.field_text(FormField::Disposition)))?;
        crate::require(
            matches!(self.field_text(FormField::Mandatory).as_str(), "yes" | "no"),
            "mandatory must be yes or no",
        )?;
        spec.mandatory = self.field_text(FormField::Mandatory) == "yes";
        spec.position = self
            .field_text(FormField::Position)
            .parse()
            .map_err(|_| crate::Error::Invalid("position must be a nonnegative integer".into()))?;
        Ok(())
    }

    fn field_text(&self, field: FormField) -> String {
        self.fields[field as usize].text.trim().to_owned()
    }

    fn field_ids(&self, field: FormField) -> Vec<String> {
        self.field_text(field)
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

fn form_values(spec: &EntrySpec) -> Vec<String> {
    vec![
        spec.id.clone(),
        spec.interpretation.clone(),
        spec.qualifiers.join("\n"),
        spec.criteria
            .iter()
            .map(criterion_text)
            .collect::<Vec<_>>()
            .join("\n"),
        spec.parent.clone().unwrap_or_default(),
        spec.depends_on.join(", "),
        spec.implementation_owners.join(", "),
        spec.verification_owners.join(", "),
        spec.task_ids.join(", "),
        serde_json::to_value(spec.disposition)
            .unwrap()
            .as_str()
            .unwrap()
            .into(),
        String::new(),
        String::new(),
        if spec.mandatory { "yes" } else { "no" }.into(),
        spec.position.to_string(),
    ]
}

fn criterion_text(criterion: &Criterion) -> String {
    format!(
        "{} | {} | {} | {}",
        criterion.id,
        if criterion.mandatory {
            "required"
        } else {
            "optional"
        },
        if criterion.integration {
            "integration"
        } else {
            "check"
        },
        criterion.text
    )
}

fn parse_criteria(text: &str) -> crate::Result<Vec<Criterion>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_criterion)
        .collect()
}

fn parse_criterion(line: &str) -> crate::Result<Criterion> {
    let parts: Vec<_> = line.splitn(4, '|').map(str::trim).collect();
    crate::require(
        parts.len() == 4
            && matches!(parts[1], "required" | "optional")
            && matches!(parts[2], "integration" | "check"),
        "criterion format: ID | required/optional | integration/check | text",
    )?;
    Ok(Criterion {
        id: parts[0].into(),
        text: parts[3].into(),
        mandatory: parts[1] == "required",
        integration: parts[2] == "integration",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> EntrySpec {
        serde_json::from_value(json!({
            "id":"req-1", "source":{"source_id":"s1","start":0,"end":5},
            "interpretation":"Keep intent", "criteria":[{"id":"C1","text":"Works"}],
            "qualifiers":["Unicode safe"], "position":2
        }))
        .unwrap()
    }

    #[test]
    fn form_round_trip_validates_identity_criteria_and_position() {
        let original = spec();
        let mut form = Form::new(original.clone(), 7);
        assert_eq!(form.spec().unwrap(), original);
        form.fields[FormField::Id as usize].text = "changed".into();
        assert!(
            form.spec()
                .unwrap_err()
                .to_string()
                .contains("identity is immutable")
        );
        form.fields[FormField::Id as usize].text = original.id;
        form.fields[FormField::Criteria as usize].text = "C2 | required | check".into();
        assert!(
            form.spec()
                .unwrap_err()
                .to_string()
                .contains("criterion format")
        );
        form.fields[FormField::Criteria as usize].text = "C2 | required | check | Works".into();
        form.fields[FormField::Position as usize].text = "-1".into();
        assert!(form.spec().is_err());
    }
}
