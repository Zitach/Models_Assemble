use std::collections::HashMap;

use ma_core::normalized::{NormalizedContent, ToolChoice, ToolDef};
use serde_json::json;

pub fn anthropic_tool_def_to_normalized(anthropic_tool: &serde_json::Value) -> Option<ToolDef> {
    let name = anthropic_tool.get("name")?.as_str()?.to_string();
    let description = anthropic_tool
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let input_schema = anthropic_tool.get("input_schema")?.clone();

    let mut tool: ToolDef = serde_json::from_value(json!({
        "name": name,
        "description": description,
        "input_schema": input_schema,
    }))
    .ok()?;
    tool.extra = HashMap::new();
    Some(tool)
}

pub fn openai_tool_def_to_normalized(openai_tool: &serde_json::Value) -> Option<ToolDef> {
    let function = openai_tool.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let description = function
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let input_schema = function.get("parameters")?.clone();

    let mut tool: ToolDef = serde_json::from_value(json!({
        "name": name,
        "description": description,
        "input_schema": input_schema,
    }))
    .ok()?;
    tool.extra = HashMap::new();
    Some(tool)
}

pub fn normalized_tool_def_to_anthropic(tool: &ToolDef) -> serde_json::Value {
    let mut obj = json!({
        "name": tool.name,
        "input_schema": tool.input_schema,
    });
    if let Some(desc) = &tool.description {
        obj["description"] = json!(desc);
    }
    obj
}

pub fn normalized_tool_def_to_openai(tool: &ToolDef) -> serde_json::Value {
    let mut function = json!({
        "name": tool.name,
        "parameters": tool.input_schema,
    });
    if let Some(desc) = &tool.description {
        function["description"] = json!(desc);
    }
    json!({
        "type": "function",
        "function": function,
    })
}

pub fn anthropic_tool_use_to_normalized(
    anthropic_tool_use: &serde_json::Value,
) -> Option<NormalizedContent> {
    let id = anthropic_tool_use.get("id")?.as_str()?.to_string();
    let name = anthropic_tool_use.get("name")?.as_str()?.to_string();
    let input = anthropic_tool_use.get("input")?.clone();

    Some(NormalizedContent::ToolUse {
        id,
        name,
        input,
        extra: HashMap::new(),
    })
}

pub fn openai_tool_call_to_normalized(
    openai_tool_call: &serde_json::Value,
) -> Option<NormalizedContent> {
    let id = openai_tool_call.get("id")?.as_str()?.to_string();
    let function = openai_tool_call.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let arguments = function.get("arguments")?.as_str()?;
    let input: serde_json::Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));

    Some(NormalizedContent::ToolUse {
        id,
        name,
        input,
        extra: HashMap::new(),
    })
}

pub fn normalized_tool_use_to_anthropic(tool_use: &NormalizedContent) -> Option<serde_json::Value> {
    match tool_use {
        NormalizedContent::ToolUse {
            id, name, input, ..
        } => Some(json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
        })),
        _ => None,
    }
}

pub fn normalized_tool_use_to_openai(tool_use: &NormalizedContent) -> Option<serde_json::Value> {
    match tool_use {
        NormalizedContent::ToolUse {
            id, name, input, ..
        } => Some(json!({
            "id": id,
            "type": "function",
            "function": {
                "name": name,
                "arguments": input.to_string(),
            }
        })),
        _ => None,
    }
}

pub fn anthropic_tool_result_to_normalized(
    anthropic_tool_result: &serde_json::Value,
) -> Option<NormalizedContent> {
    let tool_use_id = anthropic_tool_result
        .get("tool_use_id")?
        .as_str()?
        .to_string();
    let content = anthropic_tool_result.get("content")?;

    let content_box = if let Some(text) = content.as_str() {
        Box::new(NormalizedContent::Text {
            text: text.to_string(),
        })
    } else {
        Box::new(serde_json::from_value(content.clone()).unwrap_or_else(|_| {
            NormalizedContent::Text {
                text: content.to_string(),
            }
        }))
    };

    Some(NormalizedContent::ToolResult {
        tool_use_id,
        content: content_box,
        extra: HashMap::new(),
    })
}

pub fn openai_tool_result_to_normalized(
    openai_tool_message: &serde_json::Value,
) -> Option<NormalizedContent> {
    let tool_call_id = openai_tool_message
        .get("tool_call_id")?
        .as_str()?
        .to_string();
    let content = openai_tool_message.get("content")?;

    let content_box = if let Some(text) = content.as_str() {
        Box::new(NormalizedContent::Text {
            text: text.to_string(),
        })
    } else {
        Box::new(serde_json::from_value(content.clone()).unwrap_or_else(|_| {
            NormalizedContent::Text {
                text: content.to_string(),
            }
        }))
    };

    Some(NormalizedContent::ToolResult {
        tool_use_id: tool_call_id,
        content: content_box,
        extra: HashMap::new(),
    })
}

pub fn normalized_tool_result_to_anthropic(
    tool_result: &NormalizedContent,
) -> Option<serde_json::Value> {
    match tool_result {
        NormalizedContent::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let content_str = match content.as_ref() {
                NormalizedContent::Text { text } => text.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            Some(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content_str,
            }))
        }
        _ => None,
    }
}

pub fn normalized_tool_result_to_openai(
    tool_result: &NormalizedContent,
) -> Option<serde_json::Value> {
    match tool_result {
        NormalizedContent::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let content_str = match content.as_ref() {
                NormalizedContent::Text { text } => text.clone(),
                other => serde_json::to_string(other).unwrap_or_default(),
            };
            Some(json!({
                "role": "tool",
                "tool_call_id": tool_use_id,
                "content": content_str,
            }))
        }
        _ => None,
    }
}

pub fn anthropic_system_to_openai_system(system: &NormalizedContent) -> serde_json::Value {
    let text = match system {
        NormalizedContent::Text { text } => text.clone(),
        _ => serde_json::to_string(system).unwrap_or_default(),
    };
    json!({
        "role": "system",
        "content": text,
    })
}

pub fn openai_system_message_to_anthropic_system(
    openai_system: &serde_json::Value,
) -> Option<NormalizedContent> {
    let content = openai_system.get("content")?;
    if let Some(text) = content.as_str() {
        Some(NormalizedContent::Text {
            text: text.to_string(),
        })
    } else {
        serde_json::from_value(content.clone()).ok()
    }
}

pub fn anthropic_stop_reason_to_openai(stop_reason: &str) -> String {
    match stop_reason {
        "end_turn" => "stop".to_string(),
        "max_tokens" => "length".to_string(),
        "tool_use" => "tool_calls".to_string(),
        "content_filter" => "content_filter".to_string(),
        other => other.to_string(),
    }
}

pub fn openai_stop_reason_to_anthropic(stop_reason: &str) -> String {
    match stop_reason {
        "stop" => "end_turn".to_string(),
        "length" => "max_tokens".to_string(),
        "tool_calls" => "tool_use".to_string(),
        "content_filter" => "content_filter".to_string(),
        other => other.to_string(),
    }
}

pub fn anthropic_tool_choice_to_openai(tool_choice: &ToolChoice) -> serde_json::Value {
    match tool_choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Any => json!("required"),
        ToolChoice::None => json!("none"),
        ToolChoice::Specific { name } => json!({
            "type": "function",
            "function": { "name": name }
        }),
        _ => json!("auto"),
    }
}

pub fn openai_tool_choice_to_anthropic(
    openai_tool_choice: &serde_json::Value,
) -> Option<ToolChoice> {
    if let Some(s) = openai_tool_choice.as_str() {
        match s {
            "auto" => Some(ToolChoice::Auto),
            "required" => Some(ToolChoice::Any),
            "none" => Some(ToolChoice::None),
            _ => None,
        }
    } else if let Some(obj) = openai_tool_choice.as_object() {
        if obj.get("type")?.as_str()? == "function" {
            let name = obj.get("function")?.get("name")?.as_str()?.to_string();
            Some(ToolChoice::Specific { name })
        } else {
            None
        }
    } else {
        None
    }
}

pub fn normalize_max_tokens(max_tokens: Option<u32>) -> u32 {
    max_tokens.unwrap_or(4096)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_tool_def_to_normalized_basic() {
        let raw = json!({
            "name": "get_weather",
            "description": "Get weather info",
            "input_schema": {
                "type": "object",
                "properties": { "location": { "type": "string" } }
            }
        });
        let tool = anthropic_tool_def_to_normalized(&raw).unwrap();
        assert_eq!(tool.name, "get_weather");
        assert_eq!(tool.description, Some("Get weather info".to_string()));
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn openai_tool_def_to_normalized_basic() {
        let raw = json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather info",
                "parameters": {
                    "type": "object",
                    "properties": { "location": { "type": "string" } }
                }
            }
        });
        let tool = openai_tool_def_to_normalized(&raw).unwrap();
        assert_eq!(tool.name, "get_weather");
        assert_eq!(tool.description, Some("Get weather info".to_string()));
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn normalized_tool_def_to_anthropic_round_trip() {
        let tool: ToolDef = serde_json::from_value(json!({
            "name": "calc",
            "description": "Calculator",
            "input_schema": {"type": "object"},
        }))
        .unwrap();
        let anthropic = normalized_tool_def_to_anthropic(&tool);
        assert_eq!(anthropic["name"], "calc");
        assert_eq!(anthropic["description"], "Calculator");
        assert_eq!(anthropic["input_schema"]["type"], "object");

        let back = anthropic_tool_def_to_normalized(&anthropic).unwrap();
        assert_eq!(back.name, "calc");
        assert_eq!(back.description, Some("Calculator".to_string()));
    }

    #[test]
    fn normalized_tool_def_to_openai_round_trip() {
        let tool: ToolDef = serde_json::from_value(json!({
            "name": "calc",
            "description": "Calculator",
            "input_schema": {"type": "object"},
        }))
        .unwrap();
        let openai = normalized_tool_def_to_openai(&tool);
        assert_eq!(openai["type"], "function");
        assert_eq!(openai["function"]["name"], "calc");
        assert_eq!(openai["function"]["description"], "Calculator");
        assert_eq!(openai["function"]["parameters"]["type"], "object");

        let back = openai_tool_def_to_normalized(&openai).unwrap();
        assert_eq!(back.name, "calc");
        assert_eq!(back.description, Some("Calculator".to_string()));
    }

    #[test]
    fn anthropic_tool_use_to_normalized_basic() {
        let raw = json!({
            "type": "tool_use",
            "id": "toolu_123",
            "name": "get_weather",
            "input": { "location": "NYC" }
        });
        let content = anthropic_tool_use_to_normalized(&raw).unwrap();
        match content {
            NormalizedContent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "toolu_123");
                assert_eq!(name, "get_weather");
                assert_eq!(input["location"], "NYC");
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn openai_tool_call_to_normalized_basic() {
        let raw = json!({
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "get_weather",
                "arguments": "{\"location\":\"NYC\"}"
            }
        });
        let content = openai_tool_call_to_normalized(&raw).unwrap();
        match content {
            NormalizedContent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "get_weather");
                assert_eq!(input["location"], "NYC");
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn normalized_tool_use_to_anthropic_basic() {
        let tool_use = NormalizedContent::ToolUse {
            id: "toolu_123".to_string(),
            name: "get_weather".to_string(),
            input: json!({ "location": "NYC" }),
            extra: HashMap::new(),
        };
        let anthropic = normalized_tool_use_to_anthropic(&tool_use).unwrap();
        assert_eq!(anthropic["type"], "tool_use");
        assert_eq!(anthropic["id"], "toolu_123");
        assert_eq!(anthropic["name"], "get_weather");
        assert_eq!(anthropic["input"]["location"], "NYC");
    }

    #[test]
    fn normalized_tool_use_to_openai_basic() {
        let tool_use = NormalizedContent::ToolUse {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            input: json!({ "location": "NYC" }),
            extra: HashMap::new(),
        };
        let openai = normalized_tool_use_to_openai(&tool_use).unwrap();
        assert_eq!(openai["id"], "call_123");
        assert_eq!(openai["type"], "function");
        assert_eq!(openai["function"]["name"], "get_weather");
        assert_eq!(openai["function"]["arguments"], "{\"location\":\"NYC\"}");
    }

    #[test]
    fn tool_use_round_trip_anthropic() {
        let original = json!({
            "type": "tool_use",
            "id": "toolu_abc",
            "name": "calc",
            "input": { "expr": "2+2" }
        });
        let normalized = anthropic_tool_use_to_normalized(&original).unwrap();
        let back = normalized_tool_use_to_anthropic(&normalized).unwrap();
        assert_eq!(back["type"], "tool_use");
        assert_eq!(back["id"], "toolu_abc");
        assert_eq!(back["name"], "calc");
        assert_eq!(back["input"]["expr"], "2+2");
    }

    #[test]
    fn tool_use_round_trip_openai() {
        let original = json!({
            "id": "call_abc",
            "type": "function",
            "function": {
                "name": "calc",
                "arguments": "{\"expr\":\"2+2\"}"
            }
        });
        let normalized = openai_tool_call_to_normalized(&original).unwrap();
        let back = normalized_tool_use_to_openai(&normalized).unwrap();
        assert_eq!(back["id"], "call_abc");
        assert_eq!(back["type"], "function");
        assert_eq!(back["function"]["name"], "calc");
        assert_eq!(back["function"]["arguments"], "{\"expr\":\"2+2\"}");
    }

    #[test]
    fn anthropic_tool_result_to_normalized_basic() {
        let raw = json!({
            "type": "tool_result",
            "tool_use_id": "toolu_123",
            "content": "The weather is sunny",
            "is_error": false
        });
        let content = anthropic_tool_result_to_normalized(&raw).unwrap();
        match content {
            NormalizedContent::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_123");
                match content.as_ref() {
                    NormalizedContent::Text { text } => assert_eq!(text, "The weather is sunny"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn openai_tool_result_to_normalized_basic() {
        let raw = json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "The weather is sunny"
        });
        let content = openai_tool_result_to_normalized(&raw).unwrap();
        match content {
            NormalizedContent::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "call_123");
                match content.as_ref() {
                    NormalizedContent::Text { text } => assert_eq!(text, "The weather is sunny"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn normalized_tool_result_to_anthropic_basic() {
        let tool_result = NormalizedContent::ToolResult {
            tool_use_id: "toolu_123".to_string(),
            content: Box::new(NormalizedContent::Text {
                text: "The weather is sunny".to_string(),
            }),
            extra: HashMap::new(),
        };
        let anthropic = normalized_tool_result_to_anthropic(&tool_result).unwrap();
        assert_eq!(anthropic["type"], "tool_result");
        assert_eq!(anthropic["tool_use_id"], "toolu_123");
        assert_eq!(anthropic["content"], "The weather is sunny");
    }

    #[test]
    fn normalized_tool_result_to_openai_basic() {
        let tool_result = NormalizedContent::ToolResult {
            tool_use_id: "call_123".to_string(),
            content: Box::new(NormalizedContent::Text {
                text: "The weather is sunny".to_string(),
            }),
            extra: HashMap::new(),
        };
        let openai = normalized_tool_result_to_openai(&tool_result).unwrap();
        assert_eq!(openai["role"], "tool");
        assert_eq!(openai["tool_call_id"], "call_123");
        assert_eq!(openai["content"], "The weather is sunny");
    }

    #[test]
    fn tool_result_round_trip_anthropic() {
        let original = json!({
            "type": "tool_result",
            "tool_use_id": "toolu_xyz",
            "content": "Result: 42",
            "is_error": false
        });
        let normalized = anthropic_tool_result_to_normalized(&original).unwrap();
        let back = normalized_tool_result_to_anthropic(&normalized).unwrap();
        assert_eq!(back["type"], "tool_result");
        assert_eq!(back["tool_use_id"], "toolu_xyz");
        assert_eq!(back["content"], "Result: 42");
    }

    #[test]
    fn tool_result_round_trip_openai() {
        let original = json!({
            "role": "tool",
            "tool_call_id": "call_xyz",
            "content": "Result: 42"
        });
        let normalized = openai_tool_result_to_normalized(&original).unwrap();
        let back = normalized_tool_result_to_openai(&normalized).unwrap();
        assert_eq!(back["role"], "tool");
        assert_eq!(back["tool_call_id"], "call_xyz");
        assert_eq!(back["content"], "Result: 42");
    }

    #[test]
    fn anthropic_system_to_openai_system_basic() {
        let system = NormalizedContent::Text {
            text: "You are helpful.".to_string(),
        };
        let openai = anthropic_system_to_openai_system(&system);
        assert_eq!(openai["role"], "system");
        assert_eq!(openai["content"], "You are helpful.");
    }

    #[test]
    fn openai_system_message_to_anthropic_system_basic() {
        let openai = json!({
            "role": "system",
            "content": "You are helpful."
        });
        let anthropic = openai_system_message_to_anthropic_system(&openai).unwrap();
        match anthropic {
            NormalizedContent::Text { text } => assert_eq!(text, "You are helpful."),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn stop_reason_mappings() {
        assert_eq!(anthropic_stop_reason_to_openai("end_turn"), "stop");
        assert_eq!(anthropic_stop_reason_to_openai("max_tokens"), "length");
        assert_eq!(anthropic_stop_reason_to_openai("tool_use"), "tool_calls");
        assert_eq!(
            anthropic_stop_reason_to_openai("content_filter"),
            "content_filter"
        );
        assert_eq!(anthropic_stop_reason_to_openai("unknown"), "unknown");

        assert_eq!(openai_stop_reason_to_anthropic("stop"), "end_turn");
        assert_eq!(openai_stop_reason_to_anthropic("length"), "max_tokens");
        assert_eq!(openai_stop_reason_to_anthropic("tool_calls"), "tool_use");
        assert_eq!(
            openai_stop_reason_to_anthropic("content_filter"),
            "content_filter"
        );
        assert_eq!(openai_stop_reason_to_anthropic("unknown"), "unknown");
    }

    #[test]
    fn tool_choice_mappings() {
        assert_eq!(
            anthropic_tool_choice_to_openai(&ToolChoice::Auto),
            json!("auto")
        );
        assert_eq!(
            anthropic_tool_choice_to_openai(&ToolChoice::Any),
            json!("required")
        );
        assert_eq!(
            anthropic_tool_choice_to_openai(&ToolChoice::None),
            json!("none")
        );
        assert_eq!(
            anthropic_tool_choice_to_openai(&ToolChoice::Specific {
                name: "calc".to_string()
            }),
            json!({"type": "function", "function": {"name": "calc"}})
        );

        assert_eq!(
            openai_tool_choice_to_anthropic(&json!("auto")).unwrap(),
            ToolChoice::Auto
        );
        assert_eq!(
            openai_tool_choice_to_anthropic(&json!("required")).unwrap(),
            ToolChoice::Any
        );
        assert_eq!(
            openai_tool_choice_to_anthropic(&json!("none")).unwrap(),
            ToolChoice::None
        );
        assert_eq!(
            openai_tool_choice_to_anthropic(
                &json!({"type": "function", "function": {"name": "calc"}})
            )
            .unwrap(),
            ToolChoice::Specific {
                name: "calc".to_string()
            }
        );
    }

    #[test]
    fn tool_choice_round_trip() {
        let choices = vec![
            ToolChoice::Auto,
            ToolChoice::Any,
            ToolChoice::None,
            ToolChoice::Specific {
                name: "test".to_string(),
            },
        ];
        for choice in choices {
            let openai = anthropic_tool_choice_to_openai(&choice);
            let back = openai_tool_choice_to_anthropic(&openai).unwrap();
            assert_eq!(back, choice);
        }
    }

    #[test]
    fn max_tokens_normalization() {
        assert_eq!(normalize_max_tokens(Some(1024)), 1024);
        assert_eq!(normalize_max_tokens(Some(0)), 0);
        assert_eq!(normalize_max_tokens(None), 4096);
    }

    #[test]
    fn complex_tool_use_with_nested_input() {
        let anthropic = json!({
            "type": "tool_use",
            "id": "toolu_complex",
            "name": "search",
            "input": {
                "query": "rust programming",
                "filters": {
                    "language": "en",
                    "date_range": { "from": "2024-01-01", "to": "2024-12-31" }
                },
                "limit": 10
            }
        });
        let normalized = anthropic_tool_use_to_normalized(&anthropic).unwrap();
        let openai = normalized_tool_use_to_openai(&normalized).unwrap();
        let back = openai_tool_call_to_normalized(&openai).unwrap();
        let anthropic_back = normalized_tool_use_to_anthropic(&back).unwrap();
        assert_eq!(anthropic_back["input"]["query"], "rust programming");
        assert_eq!(anthropic_back["input"]["filters"]["language"], "en");
        assert_eq!(anthropic_back["input"]["limit"], 10);
    }

    #[test]
    fn complex_tool_result_with_json_content() {
        let openai = json!({
            "role": "tool",
            "tool_call_id": "call_json",
            "content": "{\"status\":\"ok\",\"data\":{\"items\":[1,2,3]}}"
        });
        let normalized = openai_tool_result_to_normalized(&openai).unwrap();
        let anthropic = normalized_tool_result_to_anthropic(&normalized).unwrap();
        assert_eq!(anthropic["tool_use_id"], "call_json");
        assert_eq!(
            anthropic["content"],
            "{\"status\":\"ok\",\"data\":{\"items\":[1,2,3]}}"
        );
    }

    #[test]
    fn preserves_tool_use_id_correlation() {
        let anthropic_tool_use = json!({
            "type": "tool_use",
            "id": "toolu_correlate_123",
            "name": "action",
            "input": {}
        });
        let normalized = anthropic_tool_use_to_normalized(&anthropic_tool_use).unwrap();
        let openai = normalized_tool_use_to_openai(&normalized).unwrap();
        assert_eq!(openai["id"], "toolu_correlate_123");

        let openai_result = json!({
            "role": "tool",
            "tool_call_id": "toolu_correlate_123",
            "content": "done"
        });
        let normalized_result = openai_tool_result_to_normalized(&openai_result).unwrap();
        match normalized_result {
            NormalizedContent::ToolResult { tool_use_id, .. } => {
                assert_eq!(tool_use_id, "toolu_correlate_123");
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn handles_empty_arguments() {
        let openai = json!({
            "id": "call_empty",
            "type": "function",
            "function": {
                "name": "noop",
                "arguments": "{}"
            }
        });
        let normalized = openai_tool_call_to_normalized(&openai).unwrap();
        match normalized {
            NormalizedContent::ToolUse { input, .. } => {
                assert_eq!(input, json!({}));
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }
    }

    #[test]
    fn handles_invalid_arguments_json() {
        let openai = json!({
            "id": "call_bad",
            "type": "function",
            "function": {
                "name": "bad",
                "arguments": "not valid json"
            }
        });
        let normalized = openai_tool_call_to_normalized(&openai).unwrap();
        match normalized {
            NormalizedContent::ToolUse { input, .. } => {
                assert_eq!(input, json!({}));
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }
    }
}
