use super::*;

/// Which relations the graph draws; each is switched on and off by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relations {
    pub depends: bool,
    pub verifies: bool,
    /// Messages to and from the selected card, and any sent just now.
    pub mail: bool,
    pub supervises: bool,
}

impl Default for Relations {
    fn default() -> Self {
        Self {
            depends: true,
            verifies: true,
            mail: true,
            supervises: false,
        }
    }
}

/// The relations in the order of their keys, 1 to 4.
pub const RELATIONS: [&str; 4] = ["dependencies", "verification", "messages", "supervision"];

impl Relations {
    pub fn all() -> Self {
        Self {
            depends: true,
            verifies: true,
            mail: true,
            supervises: true,
        }
    }

    /// Only the relation with key `index` (0 to 3).
    pub fn only(index: usize) -> Self {
        let mut relations = Self {
            depends: false,
            verifies: false,
            mail: false,
            supervises: false,
        };
        relations.toggle(index);
        relations
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(flag) = self.flag(index) {
            *flag = !*flag;
        }
    }

    pub fn shown(self, index: usize) -> bool {
        [self.depends, self.verifies, self.mail, self.supervises]
            .get(index)
            .copied()
            .unwrap_or(false)
    }

    fn flag(&mut self, index: usize) -> Option<&mut bool> {
        match index {
            0 => Some(&mut self.depends),
            1 => Some(&mut self.verifies),
            2 => Some(&mut self.mail),
            3 => Some(&mut self.supervises),
            _ => None,
        }
    }

    /// The relations drawn, by their initials, for the panel title.
    pub fn name(self) -> String {
        let shown: Vec<_> = ["D", "V", "M", "S"]
            .into_iter()
            .enumerate()
            .filter(|(index, _)| self.shown(*index))
            .map(|(_, initial)| initial)
            .collect();
        if shown.is_empty() {
            "none".into()
        } else {
            shown.join("")
        }
    }

    /// Whether `edge` is drawn; with a selection, only its own messages
    /// and recent ones are.
    pub(super) fn shows(self, edge: &Edge, selected: Option<&str>) -> bool {
        match edge.kind {
            Kind::Depends => self.depends,
            Kind::Verifies => self.verifies,
            Kind::Mail { recent } => {
                self.mail && selected.is_none_or(|key| recent || edge.from == key || edge.to == key)
            }
            Kind::Supervises => self.supervises,
        }
    }
}

/// Which cards the graph shows: those matching the text query, in one of
/// `states` and run by one of `providers`; an empty set keeps every card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub query: Option<String>,
    pub states: std::collections::BTreeSet<Health>,
    pub providers: std::collections::BTreeSet<String>,
}

impl Scope {
    pub fn is_filtered(&self) -> bool {
        !self.states.is_empty() || !self.providers.is_empty()
    }
}
