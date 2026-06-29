use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

// ─── 辅助函数：递归收集 .md 文件相对路径 ────────────────────────────────────────

fn collect_md_files(dir: &Path, root: &Path, result: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // 跳过符号链接
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if path.is_dir() {
            // 跳过以 . 开头的隐藏目录
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with('.')
            {
                collect_md_files(&path, root, result)?;
            }
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            if let Ok(rel) = path.strip_prefix(root) {
                result.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

// ─── NoteListTool ──────────────────────────────────────────────────────────────

pub struct NoteListTool {
    manifest: ToolManifest,
}

impl NoteListTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.list".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "列出工作区内所有 Markdown 笔记文件的相对路径".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "include_private": { "type": "boolean" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" } },
                        "total": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["note".to_string(), "list".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteListTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();
        let include_private = input
            .get("include_private")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let root = ctx.workspace_root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
            let mut all_paths = Vec::new();
            collect_md_files(&root, &root, &mut all_paths)
                .map_err(|e| format!("failed to list notes: {e}"))?;

            if include_private {
                Ok(all_paths)
            } else {
                let filtered: Vec<String> = all_paths
                    .into_iter()
                    .filter(|rel| {
                        let full_path = root.join(rel);
                        !crate::note_privacy::peek_kf_private_from_md_file(&full_path)
                    })
                    .collect();
                Ok(filtered)
            }
        })
        .await;

        let paths = match result {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e,
                        retryable: true,
                        cause: None,
                    },
                }
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e.to_string(),
                        retryable: true,
                        cause: None,
                    },
                }
            }
        };

        let total = paths.len();
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: serde_json::json!({ "paths": paths, "total": total }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}

// ─── NoteReadTool ──────────────────────────────────────────────────────────────

pub struct NoteReadTool {
    manifest: ToolManifest,
}

impl NoteReadTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.read".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "读取指定 Markdown 笔记的完整内容".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path"],
                    "properties": {
                        "rel_path": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rel_path": { "type": "string" },
                        "content": { "type": "string" },
                        "size_bytes": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["note".to_string(), "read".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteReadTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "rel_path is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        // 路径安全性校验
        if let Err(e) = crate::note_privacy::validate_workspace_rel_path(&rel_path) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: e,
                    retryable: false,
                    cause: None,
                },
            };
        }

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();

        let result =
            tauri::async_runtime::spawn_blocking(move || -> Result<(String, usize), String> {
                let full_path = root.join(&rel);

                // 确认文件存在
                if !full_path.exists() {
                    return Err("note not found".to_string());
                }

                // 检查是否私密
                if crate::note_privacy::peek_kf_private_from_md_file(&full_path) {
                    return Err("__PRIVACY_BLOCKED__".to_string());
                }

                let content = std::fs::read_to_string(&full_path)
                    .map_err(|e| format!("failed to read file: {e}"))?;
                let size_bytes = content.len();
                Ok((content, size_bytes))
            })
            .await;

        let (content, size_bytes) = match result {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) if e == "__PRIVACY_BLOCKED__" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::PrivacyBlocked,
                        message: "note is marked as private".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) if e == "note not found" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NotFound,
                        message: format!("note not found: {rel_path}"),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e,
                        retryable: true,
                        cause: None,
                    },
                }
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e.to_string(),
                        retryable: true,
                        cause: None,
                    },
                }
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: serde_json::json!({
                "rel_path": rel_path,
                "content": content,
                "size_bytes": size_bytes
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}

// ─── NoteWriteSectionTool ─────────────────────────────────────────────────────

pub struct NoteWriteSectionTool {
    manifest: ToolManifest,
}

impl NoteWriteSectionTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.write_section".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "覆写笔记中指定标题（heading）对应的章节内容。仅修改该 heading 到下一个同级或更高级 heading 之间的内容。".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path", "heading", "content"],
                    "properties": {
                        "rel_path": { "type": "string", "description": "笔记相对路径" },
                        "heading": { "type": "string", "description": "目标 heading 文本（不含 # 前缀）" },
                        "content": { "type": "string", "description": "替换后的章节内容（不含 heading 行本身）" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rel_path": { "type": "string" },
                        "heading": { "type": "string" },
                        "bytes_written": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read, Effect::Write],
                risk: Risk::Dangerous,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::ConfirmEach,
                examples: vec![],
                tags: vec!["note".to_string(), "write".to_string()],
                deprecated: None,
            },
        }
    }
}

/// 在 Markdown 内容中查找指定 heading 的位置，返回 (heading行起始偏移, section body 起始偏移, section body 结束偏移)。
fn find_heading_section(content: &str, target_heading: &str) -> Option<(usize, usize, usize)> {
    let target = target_heading.trim();
    let mut heading_start = None;
    let mut heading_level = 0usize;
    let mut body_start = 0usize;

    for (line_start, line) in line_offsets(content) {
        let trimmed = line.trim_start();
        if let Some(level) = parse_heading_level(trimmed) {
            let text = trimmed[level..].trim_start_matches(' ').trim();
            if heading_start.is_none() {
                if text.eq_ignore_ascii_case(target) {
                    heading_start = Some(line_start);
                    heading_level = level;
                    body_start = line_start + line.len() + 1; // +1 for '\n'
                    if body_start > content.len() {
                        body_start = content.len();
                    }
                }
            } else if level <= heading_level {
                return Some((heading_start.unwrap(), body_start, line_start));
            }
        }
    }

    heading_start.map(|hs| (hs, body_start, content.len()))
}

fn parse_heading_level(line: &str) -> Option<usize> {
    let count = line.chars().take_while(|c| *c == '#').count();
    if count > 0 && count <= 6 && line.len() > count && line.as_bytes()[count] == b' ' {
        Some(count)
    } else {
        None
    }
}

fn line_offsets(content: &str) -> Vec<(usize, &str)> {
    let mut result = Vec::new();
    let mut offset = 0;
    for line in content.split('\n') {
        result.push((offset, line));
        offset += line.len() + 1;
    }
    result
}

#[async_trait]
impl Tool for NoteWriteSectionTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteWrite
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return err_invalid_input("rel_path is required"),
        };
        let heading = match input.get("heading").and_then(|v| v.as_str()) {
            Some(h) => h.to_string(),
            None => return err_invalid_input("heading is required"),
        };
        let new_content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return err_invalid_input("content is required"),
        };

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();
        let heading_clone = heading.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<usize, WriteError> {
            let full_path = crate::tools::path_safety::resolve_existing_under_root(&root, &rel)
                .map_err(write_err_from_path_safety)?;

            if crate::note_privacy::peek_kf_private_from_md_file(&full_path) {
                return Err(WriteError::PrivacyBlocked);
            }

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| WriteError::Internal(format!("failed to read: {e}")))?;

            let (_, body_start, body_end) = find_heading_section(&content, &heading_clone)
                .ok_or_else(|| WriteError::HeadingNotFound(heading_clone.clone()))?;

            let mut new_file = String::with_capacity(content.len());
            new_file.push_str(&content[..body_start]);
            new_file.push_str(&new_content);
            if !new_content.ends_with('\n') {
                new_file.push('\n');
            }
            new_file.push_str(&content[body_end..]);

            let bytes_written = new_file.len();
            std::fs::write(&full_path, &new_file)
                .map_err(|e| WriteError::Internal(format!("failed to write: {e}")))?;

            Ok(bytes_written)
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(bytes_written)) => ToolResult::Ok {
                data: serde_json::json!({
                    "rel_path": rel_path,
                    "heading": heading,
                    "bytes_written": bytes_written
                }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics { duration_ms, ..Default::default() },
            },
            Ok(Err(WriteError::NotFound(p))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("note not found: {p}"),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::PrivacyBlocked)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PrivacyBlocked,
                    message: "note is marked as private".to_string(),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::HeadingNotFound(h))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("heading not found: {h}"),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::OutsideWorkspace)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: "path escapes workspace root".to_string(),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::InvalidRelPath(m))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: m,
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::AlreadyExists(p))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: format!("file already exists: {p}"),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::Internal(e))) => ToolResult::Err {
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

// ─── NoteCreateTool ───────────────────────────────────────────────────────────

pub struct NoteCreateTool {
    manifest: ToolManifest,
}

impl NoteCreateTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.create".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Create a new Markdown note file in the workspace. Use for \
                              structured, long-form content: research reports, meeting notes, \
                              technical analyses, tutorials, or any document that deserves its \
                              own file and path. NOT for short fleeting ideas (use thought.create \
                              for those).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path", "title"],
                    "properties": {
                        "rel_path": { "type": "string", "description": "新笔记的相对路径（如 notes/foo.md）" },
                        "title": { "type": "string", "description": "笔记标题" },
                        "content": { "type": "string", "description": "笔记正文（Markdown）" },
                        "tags": { "type": "array", "items": { "type": "string" }, "description": "标签列表" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rel_path": { "type": "string" },
                        "bytes_written": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Write],
                risk: Risk::Caution,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::ConfirmOncePerSession,
                examples: vec![],
                tags: vec!["note".to_string(), "create".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteCreateTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteWrite
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return err_invalid_input("rel_path is required"),
        };
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return err_invalid_input("title is required"),
        };
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags: Vec<String> = input
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if !rel_path.ends_with(".md") {
            return err_invalid_input("rel_path must end with .md");
        }

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<usize, WriteError> {
            let full_path = crate::tools::path_safety::resolve_new_under_root(&root, &rel)
                .map_err(write_err_from_path_safety)?;

            if full_path.exists() {
                return Err(WriteError::AlreadyExists(rel.clone()));
            }

            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| WriteError::Internal(format!("failed to create dir: {e}")))?;
            }

            let mut file_content = String::new();
            file_content.push_str("---\n");
            file_content.push_str(&format!("title: \"{}\"\n", title.replace('"', "\\\"")));
            if !tags.is_empty() {
                file_content.push_str("tags:\n");
                for tag in &tags {
                    file_content.push_str(&format!("  - {tag}\n"));
                }
            }
            file_content.push_str("---\n\n");
            file_content.push_str(&content);
            if !content.is_empty() && !content.ends_with('\n') {
                file_content.push('\n');
            }

            let bytes_written = file_content.len();
            std::fs::write(&full_path, &file_content)
                .map_err(|e| WriteError::Internal(format!("failed to write: {e}")))?;

            Ok(bytes_written)
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(bytes_written)) => ToolResult::Ok {
                data: serde_json::json!({
                    "rel_path": rel_path,
                    "bytes_written": bytes_written
                }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics { duration_ms, ..Default::default() },
            },
            Ok(Err(WriteError::OutsideWorkspace)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: "path escapes workspace root".to_string(),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::InvalidRelPath(m))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: m,
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::AlreadyExists(p))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: format!("file already exists: {p}"),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::Internal(e))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e,
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(e)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("{e:?}"),
                    retryable: false,
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

// ─── NoteAppendTool ───────────────────────────────────────────────────────────

pub struct NoteAppendTool {
    manifest: ToolManifest,
}

impl NoteAppendTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.append".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "在已有笔记文件末尾追加内容。适用于向文件尾部添加新段落、列表项或引用，无需读取并覆写整个文件。".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path", "content"],
                    "properties": {
                        "rel_path": { "type": "string", "description": "笔记相对路径（相对于工作区根目录）" },
                        "content": { "type": "string", "description": "要追加的内容文本" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rel_path": { "type": "string" },
                        "bytes_appended": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read, Effect::Write],
                risk: Risk::Caution,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::ConfirmOncePerSession,
                examples: vec![],
                tags: vec!["note".to_string(), "append".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteAppendTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteWrite
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return err_invalid_input("rel_path is required"),
        };
        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return err_invalid_input("content is required"),
        };

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<usize, WriteError> {
            let full_path = crate::tools::path_safety::resolve_existing_under_root(&root, &rel)
                .map_err(write_err_from_path_safety)?;

            if crate::note_privacy::peek_kf_private_from_md_file(&full_path) {
                return Err(WriteError::PrivacyBlocked);
            }

            // 读取已有内容以判断是否需要补一个换行分隔
            let existing = std::fs::read_to_string(&full_path)
                .map_err(|e| WriteError::Internal(format!("failed to read: {e}")))?;

            let needs_leading_newline = !existing.is_empty() && !existing.ends_with('\n');

            let mut payload = String::with_capacity(content.len() + 1);
            if needs_leading_newline {
                payload.push('\n');
            }
            payload.push_str(&content);

            let bytes_appended = payload.len();

            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&full_path)
                .map_err(|e| WriteError::Internal(format!("failed to open: {e}")))?;
            file.write_all(payload.as_bytes())
                .map_err(|e| WriteError::Internal(format!("failed to append: {e}")))?;
            file.sync_all()
                .map_err(|e| WriteError::Internal(format!("failed to sync: {e}")))?;

            Ok(bytes_appended)
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(bytes_appended)) => ToolResult::Ok {
                data: serde_json::json!({
                    "rel_path": rel_path,
                    "bytes_appended": bytes_appended
                }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics { duration_ms, ..Default::default() },
            },
            Ok(Err(WriteError::NotFound(p))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("note not found: {p}"),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::PrivacyBlocked)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PrivacyBlocked,
                    message: "note is marked as private".to_string(),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::OutsideWorkspace)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: "path escapes workspace root".to_string(),
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::InvalidRelPath(m))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PermissionDenied,
                    message: m,
                    retryable: false,
                    cause: None,
                },
            },
            Ok(Err(WriteError::Internal(e))) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e,
                    retryable: true,
                    cause: None,
                },
            },
            Ok(Err(e)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("{e:?}"),
                    retryable: false,
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

// ─── 辅助类型与函数 ───────────────────────────────────────────────────────────

#[derive(Debug)]
enum WriteError {
    NotFound(String),
    PrivacyBlocked,
    HeadingNotFound(String),
    AlreadyExists(String),
    OutsideWorkspace,
    InvalidRelPath(String),
    Internal(String),
}

fn write_err_from_path_safety(e: crate::tools::path_safety::PathSafetyError) -> WriteError {
    use crate::tools::path_safety::PathSafetyError as P;
    match e {
        P::InvalidRelPath(m) => WriteError::InvalidRelPath(m),
        P::NotFound(p) => WriteError::NotFound(p),
        P::OutsideWorkspace => WriteError::OutsideWorkspace,
        P::Io(m) => WriteError::Internal(m),
    }
}

fn err_invalid_input(msg: &str) -> ToolResult {
    ToolResult::Err {
        error: ToolError {
            code: ToolErrorCode::InvalidInput,
            message: msg.to_string(),
            retryable: false,
            cause: None,
        },
    }
}

