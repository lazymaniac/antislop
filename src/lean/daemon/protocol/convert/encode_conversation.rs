mod comments;
mod messages;
mod planning;
use super::super::{Request, WireRequest};

pub(super) fn encode_conversation(request: Request) -> WireRequest {
    let request = match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::AddSource {
            conversation,
            text,
        }) => return WireRequest::AddSource { conversation, text },
        Request::Conversation(crate::lean::daemon::ConversationRequest::GetSource {
            conversation,
            source_id,
        }) => {
            return WireRequest::GetSource {
                conversation,
                source_id,
            };
        }
        other => other,
    };
    messages::encode_group_messages(request)
        .or_else(|request| comments::encode_group_comments(*request))
        .or_else(|request| planning::encode_group_planning(*request))
        .unwrap_or_else(|_| unreachable!("request routed to wrong protocol group"))
}
