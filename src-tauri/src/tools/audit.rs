use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::context::{AuditEntry, AuditSink};

// ─── NullAuditSink ─────────────────────────────────────────────────────────────
// P0 阶段使用的空实现，不需要 workspace_root

pub struct NullAuditSink;

#[async_trait]
impl AuditSink for NullAuditSink {
    async fn record(&self, _entry: AuditEntry) {
        // no-op
    }
}

// ─── redact_value ──────────────────────────────────────────────────────────────
// 递归遍历 JSON Value，字符串长度 > 100 时替换为 "sha256:{hash}#len={n}"

pub fn redact_value(v: &Value) -> Value {
    match v {
        Value::String(s) if s.len() > 100 => {
            let hash = hex::encode(Sha256::digest(s.as_bytes()));
            Value::String(format!("sha256:{hash}#len={}", s.len()))
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => v.clone(),
        Value::Array(arr) => Value::Array(arr.iter().map(redact_value).collect()),
        Value::Object(obj) => {
            Value::Object(obj.iter().map(|(k, v)| (k.clone(), redact_value(v))).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redact_short_string_unchanged() {
        let v = json!("hello");
        assert_eq!(redact_value(&v), json!("hello"));
    }

    #[test]
    fn redact_long_string() {
        let long = "a".repeat(200);
        let result = redact_value(&json!(long));
        let s = result.as_str().unwrap();
        assert!(s.starts_with("sha256:"));
        assert!(s.contains("#len=200"));
    }

    #[test]
    fn redact_nested() {
        let v = json!({ "key": "a".repeat(200), "short": "ok" });
        let result = redact_value(&v);
        assert_eq!(result["short"], json!("ok"));
        assert!(result["key"].as_str().unwrap().starts_with("sha256:"));
    }
}
