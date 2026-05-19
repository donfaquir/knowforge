use serde_json::Value;

use super::types::{ToolError, ToolErrorCode};

/// 使用 jsonschema crate 校验 JSON input 是否符合工具声明的 input_schema。
/// P0 阶段工具数量少，不做 schema 编译缓存。
pub fn validate(schema: &Value, instance: &Value) -> Result<(), ToolError> {
    let validator = jsonschema::validator_for(schema).map_err(|e| ToolError {
        code: ToolErrorCode::Internal,
        message: format!("invalid tool input_schema: {e}"),
        retryable: false,
        cause: None,
    })?;

    if validator.is_valid(instance) {
        return Ok(());
    }

    // 收集第一条错误作为 message
    let first_err = validator
        .iter_errors(instance)
        .next()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "input validation failed".to_string());

    Err(ToolError {
        code: ToolErrorCode::InvalidInput,
        message: first_err,
        retryable: false,
        cause: None,
    })
}
