//! OpenAI API compatibility helpers.
//!
//! Some OpenAI(-compatible) models reject non-default parameters. We defensively detect
//! those errors and allow higher layers to retry with a reduced request body.

/// Returns true if an OpenAI(-compatible) error response indicates the `temperature`
/// parameter is unsupported and callers should retry without it.
///
/// This is intentionally tolerant:
/// - checks `status_code == 400`
/// - prefers structured JSON fields (`error.param`, `error.code`)
/// - falls back to substring matching on `error.message`
pub fn should_retry_without_temperature(status_code: u16, body_text: &str) -> bool {
    if status_code != 400 {
        return false;
    }

    let Ok(v) = serde_json::from_str::<serde_json::Value>(body_text) else {
        return body_text.to_ascii_lowercase().contains("temperature")
            && body_text.to_ascii_lowercase().contains("unsupported");
    };

    let err = &v["error"];
    let param = err["param"].as_str().unwrap_or_default();
    let code = err["code"].as_str().unwrap_or_default();
    let msg = err["message"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();

    (param == "temperature" && code == "unsupported_value")
        || (msg.contains("temperature")
            && (msg.contains("unsupported") || msg.contains("only the default")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_structured_temperature_unsupported_error() {
        let body = r#"{
          \"error\": {
            \"message\": \"Unsupported value: 'temperature' does not support 0.2 with this model. Only the default (1) value is supported.\",
            \"type\": \"invalid_request_error\",
            \"param\": \"temperature\",
            \"code\": \"unsupported_value\"
          }
        }"#;
        assert!(should_retry_without_temperature(400, body));
    }

    #[test]
    fn does_not_retry_on_non_400() {
        let body = r#"{\"error\":{\"param\":\"temperature\",\"code\":\"unsupported_value\"}}"#;
        assert!(!should_retry_without_temperature(401, body));
    }

    #[test]
    fn tolerant_to_non_json_body() {
        let body = "Unsupported value: temperature";
        assert!(should_retry_without_temperature(400, body));
    }
}
