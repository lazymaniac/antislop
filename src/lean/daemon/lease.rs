//! One frontend steers a conversation; the others watch. A frontend that
//! names itself claims the conversation with its first change and keeps it
//! while it follows events. Another frontend's changes are refused until it
//! takes control explicitly or the holder stops following. Frontends that do
//! not name themselves (scripts, one-shot `chat --message`) are not leased.
//! Leases live only in the daemon's memory: a restart frees them all.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// How long a holder keeps control without following events: a long poll
/// plus slack for a slow frontend.
const LEASE: Duration = Duration::from_secs(LONGEST_WAIT.as_secs() + 15);

pub(super) const OBSERVER: &str = "observer: another frontend controls this conversation; take control (palette: Take control) to act here";

/// What a request does to the conversation it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Access {
    /// Following events: keeps a held lease alive.
    Follow,
    /// Changes the conversation: needs the lease.
    Change,
    /// Takes the lease from whoever holds it.
    Take,
}

/// The conversation a request touches and how; reads other than following
/// events are free.
pub(super) fn scope(request: &Request) -> Option<(&str, Access)> {
    use super::{ConversationRequest as C, HistoryRequest as H, TerminalRequest as T};
    let (conversation, access) = match request {
        Request::History(H::Events { conversation, .. } | H::EventsPaged { conversation, .. }) => {
            (conversation, Access::Follow)
        }
        Request::Conversation(C::TakeControl { conversation }) => (conversation, Access::Take),
        Request::Conversation(C::Recover(request)) => (&request.conversation, Access::Change),
        Request::Conversation(C::SendToAgent { message }) => {
            (&message.conversation, Access::Change)
        }
        Request::Conversation(
            C::Comment { conversation, .. }
            | C::Send { conversation, .. }
            | C::Reject { conversation, .. }
            | C::Resume { conversation, .. }
            | C::RestartOrchestrator { conversation, .. }
            | C::Cancel { conversation, .. }
            | C::Pause { conversation, .. }
            | C::SetApprovalMode { conversation, .. }
            | C::AnswerApproval { conversation, .. }
            | C::ReviewComments { conversation, .. }
            | C::CommentStatus { conversation, .. }
            | C::Review { conversation, .. },
        )
        | Request::Terminal(T::TerminalInput { conversation, .. }) => {
            (conversation, Access::Change)
        }
        _ => return None,
    };
    Some((conversation.as_str(), access))
}

#[derive(Default)]
pub(super) struct Leases {
    held: Mutex<HashMap<String, (String, Instant)>>,
}

impl Leases {
    /// Checks and records `client`'s access; returns whether another
    /// frontend holds the conversation.
    pub(super) fn access(&self, conversation: &str, client: &str, access: Access) -> Result<bool> {
        let now = Instant::now();
        let mut held = self.held.lock().expect("lease mutex");
        let holder = held
            .get(conversation)
            .filter(|(_, seen)| now.duration_since(*seen) < LEASE)
            .map(|(holder, _)| holder.clone());
        let mine = holder.as_deref() == Some(client);
        match access {
            Access::Follow => {
                if mine {
                    held.insert(conversation.to_owned(), (client.to_owned(), now));
                }
                Ok(holder.is_some() && !mine)
            }
            Access::Change => {
                require(holder.is_none() || mine, OBSERVER)?;
                held.insert(conversation.to_owned(), (client.to_owned(), now));
                Ok(false)
            }
            Access::Take => {
                held.insert(conversation.to_owned(), (client.to_owned(), now));
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frontend_to_change_a_conversation_steers_it_until_another_takes_control() {
        let leases = Leases::default();
        assert!(!leases.access("c", "a", Access::Follow).unwrap());
        assert!(!leases.access("c", "b", Access::Follow).unwrap());
        leases.access("c", "a", Access::Change).unwrap();
        assert!(leases.access("c", "b", Access::Follow).unwrap());
        assert_eq!(
            leases
                .access("c", "b", Access::Change)
                .unwrap_err()
                .to_string(),
            OBSERVER
        );
        leases.access("c", "other", Access::Change).unwrap_err();
        leases.access("d", "b", Access::Change).unwrap();
        leases.access("c", "b", Access::Take).unwrap();
        assert!(leases.access("c", "a", Access::Follow).unwrap());
        leases.access("c", "a", Access::Change).unwrap_err();
        leases.access("c", "b", Access::Change).unwrap();
    }

    #[test]
    fn a_holder_that_stopped_following_loses_control() {
        let leases = Leases::default();
        leases.access("c", "a", Access::Change).unwrap();
        leases.held.lock().unwrap().get_mut("c").unwrap().1 = Instant::now() - LEASE;
        leases.access("c", "b", Access::Change).unwrap();
    }
}
