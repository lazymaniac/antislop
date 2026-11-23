//! Input ownership is derived, never stored alongside navigation or drafts.
//! The order preserves keyboard priority. Event adapters explicitly delegate
//! supported actions; an unsupported event belongs to the same layer and stops.
//! Approval cards remain nonmodal shortcuts. F1, application exit, the config
//! palette shortcut and the interview's conversation/palette shortcuts keep
//! their existing positions in the keyboard adapter. Native key releases are
//! still admitted only by shell's native_focused gate; other terminal events
//! remain ignored there. No owner is cached across actions or conversation swaps.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputKind {
    Key,
    Paste,
    Mouse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputOwner {
    History,
    Help,
    Decisions,
    Picker,
    Config,
    Flow,
    Filters,
    Consent,
    Recovery,
    Editor,
    Interview,
    Plan,
    Composer,
    DetailsInput,
    Navigation,
}

impl App {
    pub(super) fn input_owner(&self, kind: InputKind) -> InputOwner {
        use InputOwner::*;
        if self.plan_history.is_some() {
            History
        } else if self.help.is_some() {
            Help
        } else if self.decision_inbox.is_some() {
            Decisions
        } else if self.picker.is_some() {
            Picker
        } else if self
            .config_editor
            .as_ref()
            .is_some_and(|editor| editor.visible)
        {
            Config
        } else if self.flow_editor.is_some() {
            Flow
        } else if self.graph_filters.is_some() {
            Filters
        } else if self.recovery_consent.is_some() {
            Consent
        } else if self.unblock.is_some() {
            Recovery
        } else if self.editor.is_some() {
            // The visible form has always received keys before route_screen,
            // while an explicit comment editor owns paste and pointer events.
            if kind == InputKind::Key && self.interview_visible() {
                Interview
            } else {
                Editor
            }
        } else if self.interview_visible() {
            Interview
        } else if self.plan_view.is_some() {
            Plan
        } else if self
            .details
            .as_ref()
            .is_some_and(|details| details.editing || details.searching)
        {
            // A search can remain open behind the session composer. Paste
            // belongs to that composer; agent-message editing still wins.
            if kind == InputKind::Paste
                && self.navigation.focus == NavigationFocus::Composer
                && !self.details.as_ref().is_some_and(|details| details.editing)
            {
                Composer
            } else {
                DetailsInput
            }
        } else if self.navigation.focus == NavigationFocus::Composer {
            Composer
        } else {
            Navigation
        }
    }

    pub(in crate::lean::tui) fn native_input_blocked(&self) -> bool {
        self.input_owner(InputKind::Key) != InputOwner::Navigation
    }
}
