use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    UnknownMethod,
    NotFound,
    InvalidValue,
    Busy,
    Unavailable,
    PermissionDenied,
    DiskFull,
    Network,
    Cancelled,
    Timeout,
    VersionMismatch,
    Unsupported,
    Internal,
}

impl ErrorCode {
    pub fn retryable(self) -> bool {
        matches!(self, Self::Busy | Self::Unavailable | Self::Network | Self::Timeout | Self::Internal)
    }
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), details: None }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidValue, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("code".into(), serde_json::to_value(self.code).unwrap_or(Value::Null));
        m.insert("message".into(), Value::String(self.message.clone()));
        m.insert("retryable".into(), Value::Bool(self.code.retryable()));
        if let Some(d) = &self.details {
            m.insert("details".into(), d.clone());
        }
        Value::Object(m)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult = Result<Value, ApiError>;

#[derive(Debug, Clone)]
pub enum Incoming {
    Request { id: String, method: String, params: Value },
    Response { id: String, ok: bool, result: Value, error: Option<Value> },
    Event { topic: String, data: Value },
}

pub fn parse_envelope(mut m: Map<String, Value>) -> Result<Incoming, ApiError> {
    let kind = m.get("type").and_then(Value::as_str).ok_or_else(|| ApiError::bad("missing type"))?.to_owned();
    match kind.as_str() {
        "request" => {
            let id = m.get("id").and_then(Value::as_str).ok_or_else(|| ApiError::bad("missing id"))?;
            if id.is_empty() || id.len() > 64 {
                return Err(ApiError::bad("id must be 1 to 64 characters"));
            }
            let id = id.to_owned();
            let method = m.get("method").and_then(Value::as_str).ok_or_else(|| ApiError::bad("missing method"))?.to_owned();
            let params = match m.remove("params") {
                None | Some(Value::Null) => Value::Object(Map::new()),
                Some(v @ Value::Object(_)) => v,
                Some(_) => return Err(ApiError::bad("params must be an object")),
            };
            Ok(Incoming::Request { id, method, params })
        }
        "response" => Ok(Incoming::Response {
            id: m.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            ok: m.get("ok").and_then(Value::as_bool).unwrap_or(false),
            result: m.remove("result").unwrap_or(Value::Null),
            error: m.remove("error"),
        }),
        "event" => Ok(Incoming::Event {
            topic: m.get("topic").and_then(Value::as_str).ok_or_else(|| ApiError::bad("missing topic"))?.to_owned(),
            data: m.remove("data").unwrap_or(Value::Null),
        }),
        _ => Err(ApiError::bad("unknown type")),
    }
}

pub fn response_ok(id: &str, result: Value) -> Value {
    json!({ "v": 1, "id": id, "type": "response", "ok": true, "result": result })
}

pub fn response_err(id: &str, err: &ApiError) -> Value {
    json!({ "v": 1, "id": id, "type": "response", "ok": false, "error": err.to_json() })
}

pub fn event(topic: &str, data: Value) -> Value {
    json!({ "v": 1, "type": "event", "topic": topic, "data": data })
}

pub fn request(id: &str, method: &str, params: Value) -> Value {
    json!({ "v": 1, "id": id, "type": "request", "method": method, "params": params })
}

pub trait ParamsExt {
    fn str_field(&self, key: &str) -> Result<&str, ApiError>;
    fn opt_str(&self, key: &str) -> Option<&str>;
    fn bool_field(&self, key: &str) -> Result<bool, ApiError>;
    fn opt_bool(&self, key: &str) -> Option<bool>;
    fn opt_f64(&self, key: &str) -> Option<f64>;
    fn opt_i64(&self, key: &str) -> Option<i64>;
    fn str_list(&self, key: &str) -> Vec<String>;
}

impl ParamsExt for Value {
    fn str_field(&self, key: &str) -> Result<&str, ApiError> {
        self.get(key).and_then(Value::as_str).ok_or_else(|| ApiError::bad(format!("{key} is required")))
    }

    fn opt_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    fn bool_field(&self, key: &str) -> Result<bool, ApiError> {
        self.get(key).and_then(Value::as_bool).ok_or_else(|| ApiError::bad(format!("{key} is required")))
    }

    fn opt_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::as_bool)
    }

    fn opt_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_f64)
    }

    fn opt_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_i64)
    }

    fn str_list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn parses_request() {
        let r = parse_envelope(obj(json!({"v":1,"id":"c-1","type":"request","method":"models.list"}))).unwrap();
        match r {
            Incoming::Request { id, method, params } => {
                assert_eq!(id, "c-1");
                assert_eq!(method, "models.list");
                assert!(params.as_object().unwrap().is_empty());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_bad_request() {
        assert!(parse_envelope(obj(json!({"type":"request","method":"x"}))).is_err());
        assert!(parse_envelope(obj(json!({"type":"request","id":"1","method":"x","params":[1]}))).is_err());
        assert!(parse_envelope(obj(json!({"type":"nope"}))).is_err());
        assert!(parse_envelope(obj(json!({"id":"1"}))).is_err());
    }

    #[test]
    fn error_json_has_retryable() {
        let e = ApiError::new(ErrorCode::Network, "down").to_json();
        assert_eq!(e["code"], "network");
        assert_eq!(e["retryable"], true);
        let e = ApiError::not_found("x").to_json();
        assert_eq!(e["retryable"], false);
    }
}
