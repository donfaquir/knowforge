use async_trait::async_trait;
use serde_json::Value;

use crate::llm::memory::{AgentMemory, DomainUpdate, NewCorrection};
use crate::llm::provider::CompletionOverrides;
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
                description: "Save a user preference, knowledge, or style to persistent memory. \
                              Use category=\"correction\" (default) for behavioral rules \
                              (remember/always/never). Use category=\"knowledge\" when the user \
                              states their expertise or background. Use category=\"style\" when \
                              the user specifies communication preferences. Do NOT use for the \
                              user's intellectual ideas or topic insights — those belong in \
                              thought.create."
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
                        },
                        "category": {
                            "type": "string",
                            "enum": ["correction", "knowledge", "style"],
                            "description": "Type of memory to save. Default: correction"
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

        let category = input
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("correction")
            .to_string();

        let root = ctx.workspace_root.clone();
        let instr_clone = instruction.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let mut memory = AgentMemory::load(&root);
            let update = match category.as_str() {
                "knowledge" => crate::llm::memory::UserModelUpdate {
                    knowledge_domains: vec![DomainUpdate {
                        domain: instr_clone,
                        depth: "learning".to_string(),
                        current_focus: None,
                        motivation: Some(reason),
                        confidence: 0.5,
                    }],
                    ..Default::default()
                },
                "style" => {
                    let mut style_updates = std::collections::HashMap::new();
                    style_updates.insert("detail_preference".to_string(), Some(instr_clone));
                    crate::llm::memory::UserModelUpdate {
                        interaction_style_updates: style_updates,
                        ..Default::default()
                    }
                }
                _ => crate::llm::memory::UserModelUpdate {
                    new_corrections: vec![NewCorrection {
                        rule: instr_clone,
                        reason,
                    }],
                    ..Default::default()
                },
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
                            "description": "Describe which rule or preference to forget (semantic matching)"
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

        let root = ctx.workspace_root.clone();
        let memory = tauri::async_runtime::spawn_blocking({
            let root = root.clone();
            move || AgentMemory::load(&root)
        })
        .await
        .unwrap_or_default();

        if memory.corrections.is_empty() {
            return ToolResult::Ok {
                data: serde_json::json!({ "removed_count": 0, "removed_rules": [] }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics {
                    duration_ms: start.elapsed().as_millis() as u64,
                    ..Default::default()
                },
            };
        }

        let indices_to_remove = if let Some(ref provider) = ctx.provider {
            match llm_semantic_match(provider.as_ref(), &instruction, &memory.corrections).await {
                Ok(indices) => indices,
                Err(e) => {
                    eprintln!("[memory.forget] LLM matching failed, falling back to substring: {e}");
                    match substring_match(&instruction, &memory.corrections) {
                        Ok(indices) => indices,
                        Err(err_result) => return err_result,
                    }
                }
            }
        } else {
            match substring_match(&instruction, &memory.corrections) {
                Ok(indices) => indices,
                Err(err_result) => return err_result,
            }
        };

        let removed: Vec<String> = indices_to_remove
            .iter()
            .filter_map(|&i| memory.corrections.get(i).map(|c| c.rule.clone()))
            .collect();

        let remove_set: std::collections::HashSet<usize> = indices_to_remove.into_iter().collect();
        let result = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
            let mut memory = AgentMemory::load(&root);
            let mut idx = 0;
            memory.corrections.retain(|_| {
                let keep = !remove_set.contains(&idx);
                idx += 1;
                keep
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

const MIN_KEYWORD_LEN: usize = 3;

fn substring_match(
    instruction: &str,
    corrections: &[crate::llm::memory::MemoryCorrection],
) -> Result<Vec<usize>, ToolResult> {
    let keyword = instruction.to_lowercase();
    if keyword.chars().count() < MIN_KEYWORD_LEN {
        return Err(ToolResult::Err {
            error: ToolError {
                code: ToolErrorCode::InvalidInput,
                message: format!(
                    "instruction too short for keyword matching (min {} chars). \
                     Be more specific about which rule to forget.",
                    MIN_KEYWORD_LEN
                ),
                retryable: false,
                cause: None,
            },
        });
    }
    let indices: Vec<usize> = corrections
        .iter()
        .enumerate()
        .filter(|(_, c)| c.rule.to_lowercase().contains(&keyword))
        .map(|(i, _)| i)
        .collect();
    Ok(indices)
}

async fn llm_semantic_match(
    provider: &dyn crate::llm::provider::LlmProvider,
    instruction: &str,
    corrections: &[crate::llm::memory::MemoryCorrection],
) -> Result<Vec<usize>, String> {
    let rules_list: String = corrections
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{}. {} (reason: {})", i, c.rule, c.reason))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "The user wants to forget a preference or rule.\n\
         User instruction: \"{instruction}\"\n\n\
         Current saved rules:\n{rules_list}\n\n\
         Return a JSON object: {{\"indices\": [0, 2]}} containing the 0-based indices \
         of rules that semantically match the user's forget intent.\n\
         Only include rules clearly related to what the user wants to forget.\n\
         Return {{\"indices\": []}} if nothing matches."
    );

    let messages = vec![crate::llm::LlmChatMessage {
        role: "user".to_string(),
        content: prompt,
        ..Default::default()
    }];
    let overrides = CompletionOverrides {
        json_mode: true,
        temperature: Some(0.1),
        ..Default::default()
    };

    let response = provider.chat_completion(&messages, Some(&overrides)).await?;

    let parsed: Value = serde_json::from_str(&response)
        .map_err(|e| format!("failed to parse LLM response as JSON: {e}"))?;

    let indices = parsed
        .get("indices")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64().map(|n| n as usize))
                .filter(|&i| i < corrections.len())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(indices)
}
