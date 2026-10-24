use super::*;

/// How a task's result landed on the user's project: exactly one outcome
/// per finished card. `NotApplicable` is the n/a of a read-only task,
/// which has nothing to land.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Committed,
    Applied,
    LeftOnBranch,
    NoChanges,
    Conflict,
    UserEdits,
    InPlaceCommitted,
    InPlaceApplied,
    InPlaceUserEdits,
    NotApplicable,
    ForceApproved,
    Cancelled,
}

/// Every outcome with its card name and card label, side by side so the
/// sets can never drift: the daemon stamps `name`, a frontend reads a name
/// back with `named` and shows `label`.
pub(super) const OUTCOMES: [(Outcome, &str, &str); 12] = [
    (Outcome::Committed, "committed", "committed"),
    (Outcome::Applied, "applied", "applied uncommitted"),
    (Outcome::LeftOnBranch, "left_on_branch", "on branch"),
    (Outcome::NoChanges, "no_changes", "no changes"),
    (Outcome::Conflict, "conflict", "conflict"),
    (Outcome::UserEdits, "user_edits", "user edits"),
    (
        Outcome::InPlaceCommitted,
        "in_place_committed",
        "committed in place",
    ),
    (
        Outcome::InPlaceApplied,
        "in_place_applied",
        "applied in place",
    ),
    (
        Outcome::InPlaceUserEdits,
        "in_place_user_edits",
        "held for your edits",
    ),
    (Outcome::NotApplicable, "not_applicable", "read-only none"),
    (Outcome::ForceApproved, "force_approved", "force approved"),
    (Outcome::Cancelled, "cancelled", "cancelled"),
];

impl Outcome {
    /// The name the daemon stamps for this outcome, its snake_case name.
    pub fn name(self) -> &'static str {
        OUTCOMES
            .iter()
            .find(|(outcome, _, _)| *outcome == self)
            .map_or("not reported", |(_, name, _)| *name)
    }

    /// The card label for this outcome.
    pub fn label(self) -> &'static str {
        OUTCOMES
            .iter()
            .find(|(outcome, _, _)| *outcome == self)
            .map_or("not reported", |(_, _, label)| *label)
    }

    /// The outcome a reported name means; `None` when it names nothing.
    pub fn named(name: &str) -> Option<Self> {
        OUTCOMES
            .iter()
            .find(|(_, outcome, _)| *outcome == name)
            .map(|(outcome, _, _)| *outcome)
    }
}

/// The names a recovery carries in its events, in one place: the daemon
/// stamps them, the frontend reads them back. `UNBLOCK` names the unblock
/// key's restart, kept in the same list so the clearing rule has one source.
pub mod action {
    pub const INTEGRATE_WITH_CONSENT: &str = "integrate_with_consent";
    pub const KEEP_ON_BRANCH: &str = "keep_on_branch";
    pub const VERIFY_MANUAL_SNAPSHOT: &str = "verify_manual_snapshot";
    pub const RETRY_WITH_GUIDANCE: &str = "retry_with_guidance";
    pub const EXTEND_ROUNDS: &str = "extend_rounds";
    pub const RESTART_AGENT: &str = "restart_agent";
    pub const FORCE_APPROVE: &str = "force_approve";
    pub const CANCEL_TASK: &str = "cancel_task";
    pub const UNBLOCK: &str = "unblock";
}

/// Actions that start the work over: whatever an earlier round showed on
/// the card no longer stands once one is announced.
pub fn resets_card(action: &str) -> bool {
    matches!(
        action,
        action::RETRY_WITH_GUIDANCE | action::EXTEND_ROUNDS | action::UNBLOCK
    )
}

/// The landing a recovery event settles by itself, named as a card reads
/// it; `None` for actions whose landing only the runtime's own report knows.
pub fn recovery_outcome(action: &str) -> Option<&'static str> {
    let outcome = match action {
        action::KEEP_ON_BRANCH => Outcome::LeftOnBranch,
        action::FORCE_APPROVE => Outcome::ForceApproved,
        action::CANCEL_TASK => Outcome::Cancelled,
        _ => return None,
    };
    Some(outcome.name())
}

/// Card data belongs to exactly one round: stamped for another round it
/// is stale and must not show.
pub fn feedback_is_current(feedback: &Value, round: u64) -> bool {
    feedback["round"].as_u64() == Some(round)
}

/// How one obligation of a verdict counts, on a card and in `unaccounted`
/// alike: only a status the verdict format allows, with a note, is an
/// answer; an entry with a valid status but no note counts as `missing`.
pub(super) fn counted_status(entry: &Value) -> &'static str {
    let status = entry["status"].as_str().map(str::to_ascii_lowercase);
    let noted = entry["note"]
        .as_str()
        .is_some_and(|note| !note.trim().is_empty());
    match (noted, status.as_deref()) {
        (true, Some("ok")) => "ok",
        (true, Some("issue")) => "issue",
        (true, Some("blocked")) => "blocked",
        (true, Some("n/a")) => "na",
        _ => "missing",
    }
}

/// The coverage a card shows: counts over the task's real obligations only.
/// Ids the verdict adds that are not obligations are listed as `unknown`
/// and counted nowhere; obligations without a usable answer are `missing`.
/// The preview bounds the unknown ids: at most [`UNKNOWN_ID_LIMIT`] of them,
/// each within [`UNKNOWN_ID_BYTES`] bytes, with the number left out in
/// `unknown_more`; the verdict keeps every id whole, so nothing is lost to
/// the preview's cut.
pub fn coverage(verdict: &Verdict, obligations: &[(String, String)]) -> Value {
    let mut counts: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    for (id, _) in obligations {
        let entry = verdict
            .coverage
            .get(id.as_str())
            .cloned()
            .unwrap_or_default();
        *counts.entry(counted_status(&entry)).or_default() += 1;
    }
    let real: std::collections::HashSet<&str> =
        obligations.iter().map(|(id, _)| id.as_str()).collect();
    let unknown: Vec<&String> = verdict
        .coverage
        .keys()
        .filter(|id| !real.contains(id.as_str()))
        .collect();
    let shown: Vec<String> = unknown
        .iter()
        .take(UNKNOWN_ID_LIMIT)
        .map(|id| bound(id, UNKNOWN_ID_BYTES))
        .collect();
    serde_json::json!({
        "total": obligations.len(),
        "ok": counts.get("ok").copied().unwrap_or_default(),
        "issue": counts.get("issue").copied().unwrap_or_default(),
        "blocked": counts.get("blocked").copied().unwrap_or_default(),
        "na": counts.get("na").copied().unwrap_or_default(),
        "missing": counts.get("missing").copied().unwrap_or_default(),
        "unknown": shown,
        "unknown_more": unknown.len() - shown.len(),
    })
}
