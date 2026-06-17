use async_trait::async_trait;
use serde_json::Value;

use crate::llm::memory::{AgentMemory, MemoryCorrection, NewCorrection};
use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

// ─── MemorySaveTool ──────────────────────────────────────────────────────────

pub struct MemorySaveTool {
    manifest: ToolManifest,
}

impl MemorySaveTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "memory.save".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Save a user preference or behavioral instruction to persistent \
                              memory that survives across sessions. Use when the user says \
                              '记住/remember/以后都/always/never' or gives explicit instructions \
                              about how they want to be assisted. Do NOT use for the user's \
                              intellectual ideas or topic insights — those belong in thought.create."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["instruction"],
                    "properties": {
                        "instruction": {
                            "type": "string",
                            "description": "The rule or preference to remember (concise imperative sentence)"
                        },
                        "reason": {
                            "type": "string",
                            "description": "Why the user wants this (optional)"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "saved": { "type": "boolean" },
                        "instruction": { "type": "string" }
                    }
                }),
                effects: vec![Effect::Write],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["memory".to_string(), "save".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for MemorySaveTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Utility
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let instruction = match input.get("instruction").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "instruction is required and must not be empty".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "user preference".to_string());

        let root = ctx.workspace_root.clone();
        let instr_clone = instruction.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let mut memory = AgentMemory::load(&root);
            let update = crate::llm::memory::UserModelUpdate {
                new_corrections: vec![NewCorrection {
                    rule: instr_clone,
                    reason,
                }],
                ..Default::default()
            };
            memory.merge_user_model(update);
            memory.save(&root)
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(())) => ToolResult::Ok {
                data: serde_json::json!({
                    "saved": true,
                    "instruction": instruction
                }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics {
                    duration_ms,
                    ..Default::default()
                },
            },
            Ok(Err(e)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e,
                    retryable: true,
                    cause: None,
                },
            },
            Err(e) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e.to_string(),
                    retryable: true,
                    cause: None,
                },
            },
        }
    }
}

// ─── MemoryForgetTool ────────────────────────────────────────────────────────

pub struct MemoryForgetTool {
    manifest: ToolManifest,
}

impl MemoryForgetTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "memory.forget".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Remove a previously saved preference or instruction from persistent \
                              memory. Use when the user says '忘记/forget' or retracts a previous \
                              instruction."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["instruction"],
                    "properties": {
                        "instruction": {
                            "type": "string",
                            "description": "The rule or preference to forget (keyword match against saved rules)"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "removed_count": { "type": "integer" },
                        "removed_rules": { "type": "array", "items": { "type": "string" } }
                    }
                }),
                effects: vec![Effect::Write],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["memory".to_string(), "forget".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Utility
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let keyword = match input.get("instruction").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_lowercase(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "instruction is required and must not be empty".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let root = ctx.workspace_root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
            let mut memory = AgentMemory::load(&root);
            let mut removed = Vec::new();
            memory.corrections.retain(|c| {
                if c.rule.to_lowercase().contains(&keyword) {
                    removed.push(c.rule.clone());
                    false
                } else {
                    true
                }
            });
            memory.save(&root)?;
            Ok(removed)
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(removed)) => ToolResult::Ok {
                data: serde_json::json!({
                    "removed_count": removed.len(),
                    "removed_rules": removed
                }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics {
                    duration_ms,
                    ..Default::default()
                },
            },
            Ok(Err(e)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e,
                    retryable: true,
                    cause: None,
                },
            },
            Err(e) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e.to_string(),
                    retryable: true,
                    cause: None,
                },
            },
        }
    }
}
