mod comments;
mod messages;
mod planning;
use super::super::{Request, WireRequest};

pub(super) fn decode_conversation(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    let wire = match wire {
        WireRequest::AddSource { conversation, text } => {
            return Ok(Request::Conversation(
                crate::lean::daemon::ConversationRequest::AddSource { conversation, text },
            ));
        }
        WireRequest::GetSource {
            conversation,
            source_id,
        } => {
            return Ok(Request::Conversation(
                crate::lean::daemon::ConversationRequest::GetSource {
                    conversation,
                    source_id,
                },
            ));
        }
        other => other,
    };
    messages::decode_group_messages(wire)
        .or_else(|wire| comments::decode_group_comments(*wire))
        .or_else(|wire| planning::decode_group_planning(*wire))
}
