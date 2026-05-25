use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NormalizedRequest {
    #[serde(rename = "model")]
    pub model_alias: String,
    #[serde(default)]
    pub messages: Vec<NormalizedMessage>,
    #[serde(default, deserialize_with = "deserialize_optional_content")]
    pub system: Option<NormalizedContent>,
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl NormalizedRequest {
    pub fn new(model_alias: String) -> Self {
        Self {
            model_alias,
            messages: Vec::new(),
            system: None,
            tools: Vec::new(),
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            stream: false,
            metadata: None,
            thinking: None,
            extra: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NormalizedMessage {
    pub role: MessageRole,
    #[serde(deserialize_with = "deserialize_content")]
    pub content: NormalizedContent,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum NormalizedContent {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    ToolResult {
        tool_use_id: String,
        content: Box<NormalizedContent>,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    Mixed {
        items: Vec<NormalizedContent>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum NormalizedEvent {
    MessageStart {
        id: String,
        model: String,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    ContentBlockStart {
        index: u32,
        #[serde(flatten)]
        block: NormalizedContent,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    ContentBlockDelta {
        index: u32,
        delta: ContentDelta,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    ContentBlockStop {
        index: u32,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    MessageDelta {
        stop_reason: Option<String>,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    MessageStop {
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
    Error {
        error: crate::error::NormalizedError,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentDelta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
    },
    SignatureDelta {
        signature: String,
    },
    InputJsonDelta {
        partial_json: String,
        #[serde(flatten)]
        extra: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolChoice {
    Auto,
    Any,
    None,
    Specific { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub thinking_type: String,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn deserialize_content<'de, D>(de: D) -> Result<NormalizedContent, D::Error>
where
    D: Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(de)?;
    match &val {
        serde_json::Value::String(s) => Ok(NormalizedContent::Text { text: s.clone() }),
        serde_json::Value::Array(arr) => Ok(NormalizedContent::Mixed {
            items: arr
                .iter()
                .map(|v| serde_json::from_value(v.clone()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(serde::de::Error::custom)?,
        }),
        _ => serde_json::from_value(val).map_err(serde::de::Error::custom),
    }
}

fn deserialize_optional_content<'de, D>(de: D) -> Result<Option<NormalizedContent>, D::Error>
where
    D: Deserializer<'de>,
{
    let val: Option<serde_json::Value> = Option::deserialize(de)?;
    match val {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(NormalizedContent::Text { text: s })),
        Some(other) => Ok(Some(
            serde_json::from_value(other).map_err(serde::de::Error::custom)?,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: serde::de::DeserializeOwned + Serialize>(val: &T) -> serde_json::Value {
        let s = serde_json::to_string(val).expect("serialize");
        serde_json::from_str(&s).expect("deserialize")
    }

    #[test]
    fn anthropic_request_round_trip() {
        let raw = json!({
            "model": "claude-opus-4-20250514",
            "max_tokens": 1024,
            "stream": true,
            "system": "You are helpful.",
            "messages": [
                { "role": "user", "content": "Hello" },
                {
                    "role": "assistant",
                    "content": { "type": "text", "text": "Hi there!" }
                }
            ],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {
                    "type": "object",
                    "properties": { "location": { "type": "string" } }
                }
            }],
            "tool_choice": { "type": "auto" },
            "thinking": { "type": "enabled", "budget_tokens": 5000 },
            "metadata": { "user_id": "test-user" }
        });

        let req: NormalizedRequest = serde_json::from_value(raw).expect("parse anthropic");

        assert_eq!(req.model_alias, "claude-opus-4-20250514");
        assert_eq!(req.max_tokens, Some(1024));
        assert!(req.stream);
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.tools.len(), 1);
        assert!(matches!(req.tool_choice, Some(ToolChoice::Auto)));
        assert!(req.thinking.is_some());

        let back = round_trip(&req);
        assert_eq!(back["model"], "claude-opus-4-20250514");
        assert_eq!(back["max_tokens"], 1024);
        assert_eq!(back["stream"], true);
        assert_eq!(back["messages"].as_array().unwrap().len(), 2);
        assert_eq!(back["tool_choice"]["type"], "auto");
        assert_eq!(back["thinking"]["type"], "enabled");
    }

    #[test]
    fn anthropic_preserves_unknown_fields() {
        let raw = json!({
            "model": "claude-3",
            "max_tokens": 512,
            "some_new_field": "future_proof",
            "another_unknown": 42
        });

        let req: NormalizedRequest = serde_json::from_value(raw).expect("parse");
        assert_eq!(req.extra.get("some_new_field").unwrap(), "future_proof");
        assert_eq!(req.extra.get("another_unknown").unwrap(), 42);

        let back = round_trip(&req);
        assert_eq!(back["some_new_field"], "future_proof");
        assert_eq!(back["another_unknown"], 42);
    }

    #[test]
    fn openai_request_round_trip() {
        let raw = json!({
            "model": "gpt-4o",
            "messages": [
                { "role": "system", "content": "You are a coding assistant." },
                { "role": "user", "content": "Write a hello world" }
            ],
            "temperature": 0.7,
            "stream": false,
            "some_openai_field": true
        });

        let req: NormalizedRequest = serde_json::from_value(raw).expect("parse openai");

        assert_eq!(req.model_alias, "gpt-4o");
        assert_eq!(req.temperature, Some(0.7));
        assert!(!req.stream);
        assert_eq!(req.messages.len(), 2);
        assert!(req.extra.contains_key("some_openai_field"));

        let back = round_trip(&req);
        assert_eq!(back["model"], "gpt-4o");
        assert_eq!(back["temperature"], 0.7);
        assert_eq!(back["some_openai_field"], true);
    }

    #[test]
    fn string_content_normalizes_to_text_variant() {
        let msg: NormalizedMessage =
            serde_json::from_value(json!({ "role": "user", "content": "Hello" }))
                .expect("parse string content");
        assert_eq!(
            msg.content,
            NormalizedContent::Text {
                text: "Hello".into()
            }
        );
    }

    #[test]
    fn array_content_normalizes_to_mixed() {
        let msg: NormalizedMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello " },
                { "type": "text", "text": "world" }
            ]
        }))
        .expect("parse array content");
        match msg.content {
            NormalizedContent::Mixed { items } => assert_eq!(items.len(), 2),
            other => panic!("expected Mixed, got {:?}", other),
        }
    }

    #[test]
    fn text_content_round_trip() {
        let c: NormalizedContent =
            serde_json::from_value(json!({ "type": "text", "text": "Hello world" }))
                .expect("parse");
        assert_eq!(
            c,
            NormalizedContent::Text {
                text: "Hello world".into()
            }
        );
    }

    #[test]
    fn tool_use_content_round_trip() {
        let raw = json!({
            "type": "tool_use",
            "id": "tu_123",
            "name": "calculator",
            "input": {"expr": "2+2"}
        });

        let c: NormalizedContent = serde_json::from_value(raw).expect("parse");
        match &c {
            NormalizedContent::ToolUse { id, name, .. } => {
                assert_eq!(id, "tu_123");
                assert_eq!(name, "calculator");
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }

        let back = round_trip(&c);
        assert_eq!(back["type"], "tool_use");
        assert_eq!(back["id"], "tu_123");
    }

    #[test]
    fn tool_result_content_round_trip() {
        let raw = json!({
            "type": "tool_result",
            "tool_use_id": "tu_456",
            "content": { "type": "text", "text": "4" }
        });

        let c: NormalizedContent = serde_json::from_value(raw).expect("parse");
        match &c {
            NormalizedContent::ToolResult { tool_use_id, .. } => {
                assert_eq!(tool_use_id, "tu_456");
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }

        let back = round_trip(&c);
        assert_eq!(back["type"], "tool_result");
        assert_eq!(back["tool_use_id"], "tu_456");
    }

    #[test]
    fn thinking_content_round_trip() {
        let raw = json!({
            "type": "thinking",
            "thinking": "Let me think...",
            "signature": "sig_abc"
        });

        let c: NormalizedContent = serde_json::from_value(raw).expect("parse");
        match &c {
            NormalizedContent::Thinking {
                thinking,
                signature,
                ..
            } => {
                assert_eq!(thinking, "Let me think...");
                assert_eq!(*signature, Some("sig_abc".into()));
            }
            other => panic!("expected Thinking, got {:?}", other),
        }

        let back = round_trip(&c);
        assert_eq!(back["type"], "thinking");
    }

    #[test]
    fn image_content_round_trip() {
        let raw = json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "iVBOR..."
            }
        });

        let c: NormalizedContent = serde_json::from_value(raw).expect("parse");
        match &c {
            NormalizedContent::Image { source, .. } => {
                assert_eq!(source.source_type, "base64");
                assert_eq!(source.media_type, Some("image/png".into()));
            }
            other => panic!("expected Image, got {:?}", other),
        }

        let back = round_trip(&c);
        assert_eq!(back["type"], "image");
    }

    #[test]
    fn event_message_start_round_trip() {
        let raw = json!({
            "type": "message_start",
            "id": "msg_123",
            "model": "claude-3"
        });

        let e: NormalizedEvent = serde_json::from_value(raw).expect("parse");
        match &e {
            NormalizedEvent::MessageStart { id, model, .. } => {
                assert_eq!(id, "msg_123");
                assert_eq!(model, "claude-3");
            }
            other => panic!("expected MessageStart, got {:?}", other),
        }

        let back = round_trip(&e);
        assert_eq!(back["type"], "message_start");
    }

    #[test]
    fn event_content_block_delta_round_trip() {
        let raw = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "Hello" }
        });

        let e: NormalizedEvent = serde_json::from_value(raw).expect("parse");
        match &e {
            NormalizedEvent::ContentBlockDelta { index, delta, .. } => {
                assert_eq!(*index, 0);
                match delta {
                    ContentDelta::TextDelta { text } => assert_eq!(text, "Hello"),
                    other => panic!("expected TextDelta, got {:?}", other),
                }
            }
            other => panic!("expected ContentBlockDelta, got {:?}", other),
        }
    }

    #[test]
    fn tool_choice_variants_round_trip() {
        let cases: Vec<(serde_json::Value, ToolChoice)> = vec![
            (json!({"type": "auto"}), ToolChoice::Auto),
            (json!({"type": "any"}), ToolChoice::Any),
            (json!({"type": "none"}), ToolChoice::None),
            (
                json!({"type": "specific", "name": "calculator"}),
                ToolChoice::Specific {
                    name: "calculator".into(),
                },
            ),
        ];

        for (raw, expected) in cases {
            let tc: ToolChoice = serde_json::from_value(raw).expect("parse tool_choice");
            assert_eq!(tc, expected);
        }
    }

    #[test]
    fn minimal_request_only_model() {
        let raw = json!({ "model": "test-model" });

        let req: NormalizedRequest = serde_json::from_value(raw).expect("parse minimal");
        assert_eq!(req.model_alias, "test-model");
        assert!(req.messages.is_empty());
        assert_eq!(req.max_tokens, None);
        assert!(!req.stream);
    }

    #[test]
    fn thinking_config_round_trip() {
        let raw = json!({ "type": "enabled", "budget_tokens": 10000 });

        let tc: ThinkingConfig = serde_json::from_value(raw).expect("parse");
        assert_eq!(tc.thinking_type, "enabled");
        assert_eq!(tc.budget_tokens, Some(10000));

        let back = round_trip(&tc);
        assert_eq!(back["type"], "enabled");
        assert_eq!(back["budget_tokens"], 10000);
    }

    #[test]
    fn error_event_round_trip() {
        let raw = json!({
            "type": "error",
            "error": {
                "category": "rate_limited",
                "retryable": true,
                "http_status": 429,
                "provider_code": null,
                "safe_message": "Slow down",
                "raw_debug": null
            }
        });

        let e: NormalizedEvent = serde_json::from_value(raw).expect("parse");
        match &e {
            NormalizedEvent::Error { error, .. } => {
                assert_eq!(error.http_status, 429);
                assert!(error.retryable);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }
}
