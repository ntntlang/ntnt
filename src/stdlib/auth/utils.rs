use super::*;

pub(super) fn json_to_value_map(json: &serde_json::Value) -> Result<HashMap<String, Value>> {
    match crate::stdlib::json::json_to_intent_value(json) {
        Value::Map(map) => Ok(map),
        _ => Err(IntentError::type_error("Expected JSON object".to_string())),
    }
}

pub(super) fn json_map_to_value_map(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    for (key, val) in obj {
        map.insert(key.clone(), json_to_value(val));
    }
    map
}

fn json_to_value(json: &serde_json::Value) -> Value {
    crate::stdlib::json::json_to_intent_value(json)
}

pub(super) fn value_to_json(value: &Value) -> serde_json::Value {
    crate::stdlib::json::intent_value_to_json(value)
}

pub(super) fn value_map_to_json_string(map: &HashMap<String, Value>) -> String {
    let obj: serde_json::Map<String, serde_json::Value> = map
        .iter()
        .map(|(k, v)| (k.clone(), value_to_json(v)))
        .collect();
    serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

pub(super) fn json_string_to_value_map(json_str: &str) -> HashMap<String, Value> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Ok(map) = json_to_value_map(&json) {
            return map;
        }
    }
    HashMap::new()
}

/// Helper to create redirect response
pub(super) fn redirect_response(url: &str, cookies: Option<&str>) -> Value {
    let mut headers = HashMap::new();
    headers.insert("Location".to_string(), Value::String(url.to_string()));
    if let Some(cookie) = cookies {
        headers.insert("Set-Cookie".to_string(), Value::String(cookie.to_string()));
    }

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(302));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String("".to_string()));

    Value::Map(response)
}

/// Helper to create HTML response with no-cache headers.
/// Used for the OAuth exchange intermediate page — must never be cached
/// because it contains a single-use exchange token.
pub(super) fn html_response(body: &str) -> Value {
    let mut headers = HashMap::new();
    headers.insert(
        "Content-Type".to_string(),
        Value::String("text/html; charset=utf-8".to_string()),
    );
    headers.insert(
        "Cache-Control".to_string(),
        Value::String("no-store".to_string()),
    );

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(200));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String(body.to_string()));

    Value::Map(response)
}

/// Helper to create JSON response
pub(super) fn json_response(data: Value, status: i64) -> Value {
    let mut headers = HashMap::new();
    headers.insert(
        "Content-Type".to_string(),
        Value::String("application/json".to_string()),
    );

    let json_val = value_to_json(&data);
    let body = serde_json::to_string(&json_val).unwrap_or_else(|_| "{}".to_string());

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(status));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String(body));

    Value::Map(response)
}
