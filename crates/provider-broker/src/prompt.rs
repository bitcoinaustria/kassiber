//! The prompt text adapters send (`prompt.ts`).

use crate::protocol::{
    js_trim, message_images, BrokerFailure, BrokerImage, ChatMessage, ChatRequest,
};

/// Resuming replays only the newest user turn; the provider still holds the
/// rest. With no user turn, the trailing message.
fn prompt_messages(messages: &[ChatMessage], resumed: bool) -> &[ChatMessage] {
    if !resumed {
        return messages;
    }
    match messages.iter().rposition(|message| message.role == "user") {
        Some(index) => &messages[index..=index],
        None => &messages[messages.len().saturating_sub(1)..],
    }
}

/// `promptFromMessages(messages, resumed)`.
pub fn prompt_from_messages(messages: &[ChatMessage], resumed: bool) -> String {
    prompt_messages(messages, resumed)
        .iter()
        .filter(|message| !js_trim(&message.content).is_empty())
        .map(|message| format!("{}:\n{}", message.role.to_uppercase(), message.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `promptImages(messages, resumed)`: the attached images the prompt
/// carries, from the messages it replays.
pub fn prompt_images(
    messages: &[ChatMessage],
    resumed: bool,
) -> Result<Vec<BrokerImage>, BrokerFailure> {
    message_images(prompt_messages(messages, resumed))
}

/// `imagesNotShownNote(count)`: said in the prompt when a provider cannot
/// take the attached images, so the model does not answer as if it had seen
/// them.
pub fn images_not_shown_note(count: usize) -> String {
    format!(
        "\n\n[Kassiber: {count} attached image{} not shown to you because this agent cannot \
         receive images. Tell the user, and do not describe the image.]",
        if count == 1 { " was" } else { "s were" }
    )
}

/// `CHAT_ONLY_INSTRUCTIONS`.
pub const CHAT_ONLY_INSTRUCTIONS: &str = "You are the chat model inside Kassiber. Answer the user's message directly. Do not use provider-native tools, run commands, browse, read or write files, inspect the working directory, load provider skills, delegate, or edit anything. Use only the typed Kassiber tools explicitly supplied for this turn, and never invent their results.";

/// `systemInstructions(request)`: the fixed chat-only instructions, then the
/// daemon's, when it sent any.
pub fn system_instructions(request: &ChatRequest) -> String {
    match request.instructions() {
        Some(extra) if !js_trim(extra).is_empty() => format!("{CHAT_ONLY_INSTRUCTIONS}\n\n{extra}"),
        _ => CHAT_ONLY_INSTRUCTIONS.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ProviderId;
    use serde_json::json;

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage::new(role, content)
    }

    #[test]
    fn sends_only_the_latest_user_message_when_resuming_a_native_session() {
        // broker.test.ts.
        let messages = [
            message("user", "first"),
            message("assistant", "answer"),
            message("user", "second"),
        ];
        assert_eq!(prompt_from_messages(&messages, true), "USER:\nsecond");
        assert!(prompt_from_messages(&messages, false).contains("first"));
        assert_eq!(
            prompt_from_messages(&messages, false),
            "USER:\nfirst\n\nASSISTANT:\nanswer\n\nUSER:\nsecond"
        );
        // No user turn to find: fall back to the trailing message rather
        // than sending nothing.
        assert_eq!(
            prompt_from_messages(&[message("assistant", "only")], true),
            "ASSISTANT:\nonly"
        );
        assert_eq!(prompt_from_messages(&[], true), "");
        // Blank messages are left out.
        assert_eq!(
            prompt_from_messages(&[message("user", " \n "), message("user", "x")], false),
            "USER:\nx"
        );
    }

    const PNG: &str = "iVBORw0KGgo=";

    fn with_images(role: &str, content: &str, images: serde_json::Value) -> ChatMessage {
        serde_json::from_value(json!({"role": role, "content": content, "images": images})).unwrap()
    }

    #[test]
    fn carries_the_images_of_the_messages_the_prompt_replays() {
        // images.test.ts.
        let latest = with_images(
            "user",
            "second",
            json!([{"media_type": "image/jpeg", "data": PNG}]),
        );
        let messages = [
            with_images(
                "user",
                "first",
                json!([{"media_type": "image/png", "data": PNG}]),
            ),
            message("assistant", "answer"),
            latest,
        ];
        assert_eq!(prompt_images(&messages, false).unwrap().len(), 2);
        assert_eq!(
            prompt_images(&messages, true).unwrap(),
            [BrokerImage {
                media_type: "image/jpeg".into(),
                data: PNG.into()
            }]
        );
        assert!(prompt_images(&[message("user", "x")], false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rejects_an_image_that_is_not_inline_base64_of_a_known_type() {
        // images.test.ts, plus shapes TypeScript checks with `typeof`.
        for bad in [
            json!({"media_type": "image/svg+xml", "data": PNG}),
            json!({"media_type": "image/png", "data": "https://example.com/a.png"}),
            json!({"media_type": "image/png", "data": ""}),
            json!({"media_type": "text/plain", "data": PNG}),
            json!({"media_type": "image/png", "data": "abc==="}),
            json!({"media_type": "image/png", "data": "=="}),
            json!({"media_type": "image/png", "data": 7}),
            json!({"data": PNG}),
            json!(null),
        ] {
            let messages = [with_images("user", "x", json!([bad]))];
            assert_eq!(
                prompt_images(&messages, false).unwrap_err().message(),
                "Invalid attached image."
            );
        }
        let many: Vec<_> = (0..17)
            .map(|_| json!({"media_type": "image/png", "data": PNG}))
            .collect();
        let messages = [with_images("user", "x", json!(many))];
        assert_eq!(
            prompt_images(&messages, false).unwrap_err().message(),
            "Too many attached images."
        );
        let sixteen = [with_images("user", "x", json!(many[..16]))];
        assert_eq!(prompt_images(&sixteen, false).unwrap().len(), 16);
    }

    #[test]
    fn says_how_many_images_were_not_shown() {
        assert!(images_not_shown_note(1).contains("1 attached image was not shown"));
        assert!(images_not_shown_note(3).contains("3 attached images were not shown"));
        assert!(images_not_shown_note(1).starts_with("\n\n[Kassiber: "));
    }

    #[test]
    fn appends_only_non_blank_string_instructions() {
        let request = ChatRequest::new(ProviderId::Claude, "default", Vec::new());
        assert_eq!(system_instructions(&request), CHAT_ONLY_INSTRUCTIONS);
        let mut with = request.clone();
        with.instructions = Some(json!("Book: demo."));
        assert_eq!(
            system_instructions(&with),
            format!("{CHAT_ONLY_INSTRUCTIONS}\n\nBook: demo.")
        );
        with.instructions = Some(json!("  "));
        assert_eq!(system_instructions(&with), CHAT_ONLY_INSTRUCTIONS);
        with.instructions = Some(json!(42));
        assert_eq!(system_instructions(&with), CHAT_ONLY_INSTRUCTIONS);
    }
}
