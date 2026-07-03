use std::sync::Arc;

use crate::tools::types::*;
use crate::tools::context::ToolContext;

pub struct PlanUpdateStepTool {
    manifest: ToolManifest,
}

impl PlanUpdateStepTool {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            manifest: ToolManifest {
                name: "plan.update_step".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Report progress during plan execution. Call before starting each step (status: in_progress) and after completing it (status: done).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "step": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "The step number from the plan"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["in_progress", "done"],
                            "description": "The step status"
                        }
                    },
                    "required": ["step", "status"],
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ok": { "type": "boolean" }
                    }
                }),
                effects: vec![],
                risk: Risk::Safe,
                privacy_aware: false,
                requires_workspace: false,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["planning".to_string(), "utility".to_string()],
                deprecated: None,
            },
        })
    }
}

#[async_trait::async_trait]
impl Tool for PlanUpdateStepTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Utility
    }

    async fn invoke(&self, _ctx: &ToolContext, _input: serde_json::Value) -> ToolResult {
        ToolResult::Ok {
            data: serde_json::json!({ "ok": true }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms: 0,
                ..Default::default()
            },
        }
    }
}
