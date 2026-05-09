//! 工作区 `.knowforge/conversations/` 多会话持久化（任务 07）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KNOWFORGE_DIR: &str = ".knowforge";
const CONVERSATIONS_SUBDIR: &str = "conversations";
/// 想法管理全屏页右侧 AI：与文档侧栏会话分库存储，避免历史串台。
const THOUGHT_MGMT_CONVERSATIONS_SUBDIR: &str = "conversations_thought_mgmt";
const INDEX_FILE: &str = "index.json";
const CONVERSATIONS_SCHEMA_VERSION: u32 = 1;

fn knowforge_dir(root: &Path) -> PathBuf {
    root.join(KNOWFORGE_DIR)
}

fn conversations_dir(root: &Path, subdir: &str) -> PathBuf {
    knowforge_dir(root).join(subdir)
}

fn index_path(root: &Path, subdir: &str) -> PathBuf {
    conversations_dir(root, subdir).join(INDEX_FILE)
}

fn body_path(root: &Path, subdir: &str, id: &str) -> PathBuf {
    conversations_dir(root, subdir).join(format!("{id}.json"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn backup_corrupt(path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("failed to read corrupt file: {e}"))?;
    let parent = path.parent().ok_or_else(|| "path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("failed to create parent dir: {e}"))?;
    let ms = now_ms().max(0) as u128;
    let bak = parent.join(format!(
        "{}.broken.{ms}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file")
    ));
    fs::write(&bak, raw).map_err(|e| format!("failed to write backup: {e}"))?;
    Ok(())
}

fn atomic_write_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "invalid path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("failed to create dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let pretty = serde_json::to_string_pretty(value).map_err(|e| format!("serialize: {e}"))?;
    let contents = format!("{pretty}\n");
    fs::write(&tmp, contents).map_err(|e| format!("write temp: {e}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("remove old: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

fn parse_uuid(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id.trim()).map_err(|_| "Invalid conversation id.".to_string())?;
    Ok(())
}

/// 按文档 §4.3：首条 user 内容生成标题（48 个 Unicode 标量）
fn compute_title_from_messages(messages: &[PersistedMessageDisk]) -> String {
    const MAX_CHARS: usize = 48;
    for m in messages {
        if m.role == "user" {
            let t = m.content.trim();
            if t.is_empty() {
                return "New chat".to_string();
            }
            let n = t.chars().count();
            if n <= MAX_CHARS {
                return t.to_string();
            }
            let prefix: String = t.chars().take(MAX_CHARS).collect();
            return format!("{prefix}…");
        }
    }
    "New chat".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationMetaDisk {
    id: String,
    title: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationIndexDisk {
    #[serde(rename = "$schemaVersion")]
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_conversation_id: Option<String>,
    #[serde(default)]
    conversations: Vec<ConversationMetaDisk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedMessageDisk {
    id: String,
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_context_sources: Option<Value>,
}

/// 会话级「想法聚焦」上下文（磁盘与 LLM 字段对齐）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtFocusContextDisk {
    pub thought_id: String,
    pub thought_body: String,
    pub maturity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationBodyDisk {
    #[serde(rename = "$schemaVersion")]
    schema_version: u32,
    id: String,
    updated_at: i64,
    attach_current_note: bool,
    /// 是否在发送前检索 Vault 关键词上下文（任务 08）；旧文件缺省为 false。
    #[serde(default)]
    include_vault_context: bool,
    #[serde(default)]
    thought_focus_context: Option<ThoughtFocusContextDisk>,
    messages: Vec<PersistedMessageDisk>,
}

impl Default for ConversationIndexDisk {
    fn default() -> Self {
        Self {
            schema_version: CONVERSATIONS_SCHEMA_VERSION,
            active_conversation_id: None,
            conversations: Vec::new(),
        }
    }
}

fn read_index(root: &Path, subdir: &str) -> Result<ConversationIndexDisk, String> {
    let path = index_path(root, subdir);
    if !path.exists() {
        return Ok(ConversationIndexDisk::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("failed to read conversation index: {e}"))?;
    match serde_json::from_str::<ConversationIndexDisk>(&raw) {
        Ok(mut idx) => {
            if idx.schema_version > CONVERSATIONS_SCHEMA_VERSION {
                return Err(format!(
                    "Unsupported conversations schema version {} (expected <= {}).",
                    idx.schema_version, CONVERSATIONS_SCHEMA_VERSION
                ));
            }
            if idx.schema_version == 0 {
                idx.schema_version = CONVERSATIONS_SCHEMA_VERSION;
            }
            Ok(idx)
        }
        Err(_) => {
            backup_corrupt(&path)?;
            Ok(ConversationIndexDisk::default())
        }
    }
}

fn write_index(root: &Path, subdir: &str, index: &ConversationIndexDisk) -> Result<(), String> {
    let v = serde_json::to_value(index).map_err(|e| format!("index to json: {e}"))?;
    atomic_write_json(&index_path(root, subdir), &v)
}

fn body_exists(root: &Path, subdir: &str, id: &str) -> bool {
    body_path(root, subdir, id).is_file()
}

/// 过滤索引中 body 已缺失的条目，修正 active（§5.6）
fn repair_index_against_disk(root: &Path, subdir: &str, index: &mut ConversationIndexDisk) -> Result<bool, String> {
    let before_n = index.conversations.len();
    index
        .conversations
        .retain(|m| body_exists(root, subdir, &m.id));
    let mut changed = before_n != index.conversations.len();

    if let Some(ref aid) = index.active_conversation_id {
        if !index.conversations.iter().any(|c| c.id == *aid) {
            index.active_conversation_id = index
                .conversations
                .iter()
                .max_by_key(|c| c.updated_at)
                .map(|c| c.id.clone());
            changed = true;
        }
    }

    if changed {
        write_index(root, subdir, index)?;
    }
    Ok(changed)
}

// --- IPC 类型（camelCase，无 $ 前缀） ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMetaOut {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAiConversationsResponse {
    pub conversations: Vec<ConversationMetaOut>,
    pub active_conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedMessageOut {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_context_sources: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationBodyOut {
    pub schema_version: u32,
    pub id: String,
    pub updated_at: i64,
    pub attach_current_note: bool,
    pub include_vault_context: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_focus_context: Option<ThoughtFocusContextDisk>,
    pub messages: Vec<PersistedMessageOut>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAiConversationArgs {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thought_focus_context: Option<ThoughtFocusContextDisk>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAiConversationResponse {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadAiConversationArgs {
    pub conversation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedMessageIn {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub streaming: Option<bool>,
    #[serde(default)]
    pub reply_context_sources: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAiConversationArgs {
    pub conversation_id: String,
    pub attach_current_note: bool,
    /// 缺省 false：旧版前端未传字段时兼容。
    #[serde(default)]
    pub include_vault_context: bool,
    #[serde(default)]
    pub thought_focus_context: Option<ThoughtFocusContextDisk>,
    pub messages: Vec<PersistedMessageIn>,
    #[serde(default)]
    pub set_as_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveAiConversationArgs {
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAiConversationArgs {
    pub conversation_id: String,
}

fn meta_to_out(m: &ConversationMetaDisk) -> ConversationMetaOut {
    ConversationMetaOut {
        id: m.id.clone(),
        title: m.title.clone(),
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn normalize_role(role: &str) -> Result<String, String> {
    match role.trim() {
        "user" | "assistant" => Ok(role.trim().to_string()),
        _ => Err("Invalid message role.".to_string()),
    }
}

fn validate_messages_for_save(msgs: &[PersistedMessageIn]) -> Result<Vec<PersistedMessageDisk>, String> {
    let mut out = Vec::with_capacity(msgs.len());
    for m in msgs {
        if m.streaming == Some(true) {
            return Err("Cannot save while a message is streaming.".to_string());
        }
        let role = normalize_role(&m.role)?;
        out.push(PersistedMessageDisk {
            id: m.id.clone(),
            role,
            content: m.content.clone(),
            reply_context_sources: m.reply_context_sources.clone(),
        });
    }
    Ok(out)
}

pub fn list_ai_conversations_blocking(root: &Path) -> Result<ListAiConversationsResponse, String> {
    list_ai_conversations_for_subdir(root, CONVERSATIONS_SUBDIR)
}

pub fn list_thought_mgmt_ai_conversations_blocking(root: &Path) -> Result<ListAiConversationsResponse, String> {
    list_ai_conversations_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR)
}

fn list_ai_conversations_for_subdir(root: &Path, subdir: &str) -> Result<ListAiConversationsResponse, String> {
    let mut index = read_index(root, subdir)?;
    let _ = repair_index_against_disk(root, subdir, &mut index)?;
    let conversations: Vec<ConversationMetaOut> = index.conversations.iter().map(meta_to_out).collect();
    Ok(ListAiConversationsResponse {
        conversations,
        active_conversation_id: index.active_conversation_id.clone(),
    })
}

pub fn create_ai_conversation_blocking(root: &Path, args: CreateAiConversationArgs) -> Result<CreateAiConversationResponse, String> {
    create_ai_conversation_for_subdir(root, CONVERSATIONS_SUBDIR, args, true, false)
}

pub fn create_thought_mgmt_ai_conversation_blocking(
    root: &Path,
    args: CreateAiConversationArgs,
) -> Result<CreateAiConversationResponse, String> {
    create_ai_conversation_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, args, false, true)
}

fn create_ai_conversation_for_subdir(
    root: &Path,
    subdir: &str,
    args: CreateAiConversationArgs,
    attach_current_note: bool,
    include_vault_context: bool,
) -> Result<CreateAiConversationResponse, String> {
    fs::create_dir_all(conversations_dir(root, subdir)).map_err(|e| format!("create conversations dir: {e}"))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    let title_initial = args
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "New chat".to_string());

    let body = ConversationBodyDisk {
        schema_version: CONVERSATIONS_SCHEMA_VERSION,
        id: id.clone(),
        updated_at: now,
        attach_current_note,
        include_vault_context,
        thought_focus_context: args.thought_focus_context.clone(),
        messages: Vec::new(),
    };
    let body_v = serde_json::to_value(&body).map_err(|e| format!("body json: {e}"))?;
    atomic_write_json(&body_path(root, subdir, &id), &body_v)?;

    let mut index = read_index(root, subdir)?;
    let _ = repair_index_against_disk(root, subdir, &mut index)?;

    index.conversations.push(ConversationMetaDisk {
        id: id.clone(),
        title: title_initial,
        created_at: now,
        updated_at: now,
    });
    index.active_conversation_id = Some(id.clone());
    write_index(root, subdir, &index)?;

    Ok(CreateAiConversationResponse { id })
}

pub fn load_ai_conversation_blocking(root: &Path, args: LoadAiConversationArgs) -> Result<ConversationBodyOut, String> {
    load_ai_conversation_for_subdir(root, CONVERSATIONS_SUBDIR, args)
}

pub fn load_thought_mgmt_ai_conversation_blocking(root: &Path, args: LoadAiConversationArgs) -> Result<ConversationBodyOut, String> {
    load_ai_conversation_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, args)
}

fn load_ai_conversation_for_subdir(root: &Path, subdir: &str, args: LoadAiConversationArgs) -> Result<ConversationBodyOut, String> {
    parse_uuid(&args.conversation_id)?;
    let path = body_path(root, subdir, &args.conversation_id);
    if !path.is_file() {
        return Err("Conversation file not found.".to_string());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read conversation: {e}"))?;
    let body: ConversationBodyDisk = serde_json::from_str(&raw).map_err(|_| {
        let _ = backup_corrupt(&path);
        "Invalid conversation file.".to_string()
    })?;
    if body.schema_version > CONVERSATIONS_SCHEMA_VERSION {
        return Err("Unsupported conversation file schema.".to_string());
    }
    if body.id != args.conversation_id {
        return Err("Conversation id mismatch.".to_string());
    }
    let messages: Vec<PersistedMessageOut> = body
        .messages
        .into_iter()
        .map(|m| PersistedMessageOut {
            id: m.id,
            role: m.role,
            content: m.content,
            reply_context_sources: m.reply_context_sources,
        })
        .collect();
    Ok(ConversationBodyOut {
        schema_version: body.schema_version,
        id: body.id,
        updated_at: body.updated_at,
        attach_current_note: body.attach_current_note,
        include_vault_context: body.include_vault_context,
        thought_focus_context: body.thought_focus_context,
        messages,
    })
}

pub fn save_ai_conversation_blocking(root: &Path, args: SaveAiConversationArgs) -> Result<(), String> {
    save_ai_conversation_for_subdir(root, CONVERSATIONS_SUBDIR, args)
}

pub fn save_thought_mgmt_ai_conversation_blocking(root: &Path, args: SaveAiConversationArgs) -> Result<(), String> {
    save_ai_conversation_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, args)
}

fn save_ai_conversation_for_subdir(root: &Path, subdir: &str, args: SaveAiConversationArgs) -> Result<(), String> {
    parse_uuid(&args.conversation_id)?;
    let disk_msgs = validate_messages_for_save(&args.messages)?;
    let now = now_ms();
    let title = compute_title_from_messages(&disk_msgs);

    let body = ConversationBodyDisk {
        schema_version: CONVERSATIONS_SCHEMA_VERSION,
        id: args.conversation_id.clone(),
        updated_at: now,
        attach_current_note: args.attach_current_note,
        include_vault_context: args.include_vault_context,
        thought_focus_context: args.thought_focus_context.clone(),
        messages: disk_msgs.clone(),
    };
    let body_v = serde_json::to_value(&body).map_err(|e| format!("body json: {e}"))?;
    atomic_write_json(&body_path(root, subdir, &args.conversation_id), &body_v)?;

    let mut index = read_index(root, subdir)?;
    let _ = repair_index_against_disk(root, subdir, &mut index)?;

    let pos = index
        .conversations
        .iter()
        .position(|c| c.id == args.conversation_id)
        .ok_or_else(|| "Unknown conversation id.".to_string())?;

    index.conversations[pos].updated_at = now;
    if index.conversations[pos].title != title {
        index.conversations[pos].title = title;
    }

    let set_active = args.set_as_active.unwrap_or(true);
    if set_active {
        index.active_conversation_id = Some(args.conversation_id.clone());
    }

    write_index(root, subdir, &index)?;
    Ok(())
}

pub fn set_active_ai_conversation_blocking(root: &Path, args: SetActiveAiConversationArgs) -> Result<(), String> {
    set_active_ai_conversation_for_subdir(root, CONVERSATIONS_SUBDIR, args)
}

pub fn set_active_thought_mgmt_ai_conversation_blocking(
    root: &Path,
    args: SetActiveAiConversationArgs,
) -> Result<(), String> {
    set_active_ai_conversation_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, args)
}

fn set_active_ai_conversation_for_subdir(root: &Path, subdir: &str, args: SetActiveAiConversationArgs) -> Result<(), String> {
    if let Some(ref id) = args.conversation_id {
        parse_uuid(id)?;
        if !body_exists(root, subdir, id) {
            return Err("Conversation file not found.".to_string());
        }
    }

    let mut index = read_index(root, subdir)?;
    let _ = repair_index_against_disk(root, subdir, &mut index)?;

    index.active_conversation_id = args.conversation_id.clone();
    write_index(root, subdir, &index)?;
    Ok(())
}

pub fn delete_ai_conversation_blocking(root: &Path, args: DeleteAiConversationArgs) -> Result<(), String> {
    delete_ai_conversation_for_subdir(root, CONVERSATIONS_SUBDIR, args)
}

pub fn delete_thought_mgmt_ai_conversation_blocking(root: &Path, args: DeleteAiConversationArgs) -> Result<(), String> {
    delete_ai_conversation_for_subdir(root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, args)
}

fn delete_ai_conversation_for_subdir(root: &Path, subdir: &str, args: DeleteAiConversationArgs) -> Result<(), String> {
    parse_uuid(&args.conversation_id)?;
    let path = body_path(root, subdir, &args.conversation_id);
    if path.is_file() {
        fs::remove_file(&path).map_err(|e| format!("delete conversation file: {e}"))?;
    }

    let mut index = read_index(root, subdir)?;
    index.conversations.retain(|c| c.id != args.conversation_id);
    if index.active_conversation_id.as_deref() == Some(args.conversation_id.as_str()) {
        index.active_conversation_id = index
            .conversations
            .iter()
            .max_by_key(|c| c.updated_at)
            .map(|c| c.id.clone());
    }
    write_index(root, subdir, &index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "knowforge_conv_test_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn create_list_load_save_roundtrip() {
        let root = tmp_root();
        fs::create_dir_all(&root).unwrap();
        let id = create_ai_conversation_blocking(&root, CreateAiConversationArgs {
            title: None,
            thought_focus_context: None,
        })
            .unwrap()
            .id;
        let list = list_ai_conversations_blocking(&root).unwrap();
        assert_eq!(list.conversations.len(), 1);
        assert_eq!(list.active_conversation_id.as_deref(), Some(id.as_str()));

        let loaded = load_ai_conversation_blocking(
            &root,
            LoadAiConversationArgs {
                conversation_id: id.clone(),
            },
        )
        .unwrap();
        assert!(loaded.messages.is_empty());

        save_ai_conversation_blocking(
            &root,
            SaveAiConversationArgs {
                conversation_id: id.clone(),
                attach_current_note: false,
                include_vault_context: false,
                thought_focus_context: None,
                messages: vec![
                    PersistedMessageIn {
                        id: "u1".to_string(),
                        role: "user".to_string(),
                        content: "Hello world".to_string(),
                        streaming: None,
                        reply_context_sources: None,
                    },
                    PersistedMessageIn {
                        id: "a1".to_string(),
                        role: "assistant".to_string(),
                        content: "Hi".to_string(),
                        streaming: None,
                        reply_context_sources: None,
                    },
                ],
                set_as_active: Some(true),
            },
        )
        .unwrap();

        let list2 = list_ai_conversations_blocking(&root).unwrap();
        assert_eq!(list2.conversations[0].title, "Hello world");
        let loaded2 = load_ai_conversation_blocking(
            &root,
            LoadAiConversationArgs {
                conversation_id: id.clone(),
            },
        )
        .unwrap();
        assert_eq!(loaded2.messages.len(), 2);
        assert!(!loaded2.attach_current_note);
    }

    #[test]
    fn title_new_chat_when_no_user() {
        let msgs: Vec<PersistedMessageDisk> = vec![PersistedMessageDisk {
            id: "a".to_string(),
            role: "assistant".to_string(),
            content: "only".to_string(),
            reply_context_sources: None,
        }];
        assert_eq!(compute_title_from_messages(&msgs), "New chat");
    }

    #[test]
    fn thought_mgmt_storage_isolated_from_document_conversations() {
        let root = tmp_root();
        fs::create_dir_all(&root).unwrap();
        let doc_id = create_ai_conversation_blocking(&root, CreateAiConversationArgs {
            title: None,
            thought_focus_context: None,
        })
        .unwrap()
        .id;
        let tm_id = create_thought_mgmt_ai_conversation_blocking(&root, CreateAiConversationArgs {
            title: None,
            thought_focus_context: None,
        })
        .unwrap()
        .id;
        assert_ne!(doc_id, tm_id);

        let doc_list = list_ai_conversations_blocking(&root).unwrap();
        let tm_list = list_thought_mgmt_ai_conversations_blocking(&root).unwrap();
        assert_eq!(doc_list.conversations.len(), 1);
        assert_eq!(tm_list.conversations.len(), 1);
        assert_eq!(doc_list.conversations[0].id, doc_id);
        assert_eq!(tm_list.conversations[0].id, tm_id);

        assert!(body_path(&root, CONVERSATIONS_SUBDIR, &doc_id).is_file());
        assert!(body_path(&root, THOUGHT_MGMT_CONVERSATIONS_SUBDIR, &tm_id).is_file());
    }

    #[test]
    fn delete_removes_file_and_index() {
        let root = tmp_root();
        fs::create_dir_all(&root).unwrap();
        let id = create_ai_conversation_blocking(&root, CreateAiConversationArgs {
            title: None,
            thought_focus_context: None,
        })
            .unwrap()
            .id;
        delete_ai_conversation_blocking(
            &root,
            DeleteAiConversationArgs {
                conversation_id: id.clone(),
            },
        )
        .unwrap();
        assert!(!body_path(&root, CONVERSATIONS_SUBDIR, &id).exists());
        let list = list_ai_conversations_blocking(&root).unwrap();
        assert!(list.conversations.is_empty());
    }
}
