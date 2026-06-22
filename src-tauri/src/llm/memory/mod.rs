mod extraction;
mod injection;
mod merge;
mod types;
mod workspace;

#[cfg(test)]
mod tests;

use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::provider::LlmProvider;
use super::LlmChatMessage;

pub use extraction::{apply_single_proposal, should_reflect};
pub use types::*;
pub use workspace::observe_workspace;

const MEMORY_FILE: &str = "agent_memory.json";
const SNAPSHOT_FILE: &str = "agent_memory.snapshot.json";
const PENDING_FILE: &str = "pending_proposals.json";
const KNOWFORGE_DIR: &str = ".knowforge/memory";

// ── Load / Save ──

impl AgentMemory {
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(KNOWFORGE_DIR).join(MEMORY_FILE);
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                eprintln!("[memory] Failed to parse agent_memory.json: {e}");
                Self::default()
            }),
            Err(e) => {
                eprintln!("[memory] Failed to read agent_memory.json: {e}");
                Self::default()
            }
        }
    }

    pub fn save(&self, workspace_root: &Path) -> Result<(), String> {
        let dir = workspace_root.join(KNOWFORGE_DIR);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create .knowforge/memory dir: {e}"))?;
        let path = dir.join(MEMORY_FILE);
        let tmp = dir.join(format!("{MEMORY_FILE}.tmp"));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize memory: {e}"))?;
        std::fs::write(&tmp, format!("{json}\n"))
            .map_err(|e| format!("Failed to write temp memory: {e}"))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("Failed to finalize memory: {e}"))?;
        Ok(())
    }
}

// ── MemoryManager ──

pub struct MemoryManager {
    pub memory: AgentMemory,
    cloud: Option<Arc<dyn LlmProvider>>,
    workspace_root: PathBuf,
    dirty: bool,
    extraction_messages: Option<Vec<LlmChatMessage>>,
}

impl MemoryManager {
    pub fn new(workspace_root: PathBuf, cloud: Option<Arc<dyn LlmProvider>>) -> Self {
        let mut memory = AgentMemory::load(&workspace_root);
        memory.apply_confidence_decay();
        memory.expire_pending_styles();
        memory.expire_superseded_corrections();

        if workspace::is_workspace_stale(&memory.workspace.updated_at) {
            let note_paths = workspace::scan_md_paths(&workspace_root);
            memory.workspace = observe_workspace(&workspace_root, &note_paths);
            if let Err(e) = memory.save(&workspace_root) {
                eprintln!("[memory] Failed to save after workspace observation: {e}");
            }
        }

        Self {
            memory,
            cloud,
            workspace_root,
            dirty: false,
            extraction_messages: None,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn reset_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn set_extraction_messages(&mut self, msgs: Vec<LlmChatMessage>) {
        self.extraction_messages = Some(msgs);
    }

    pub fn take_extraction_messages(&mut self) -> Option<Vec<LlmChatMessage>> {
        self.extraction_messages.take()
    }

    pub fn format_for_injection(&self) -> Option<String> {
        self.memory.format_for_injection()
    }

    pub fn create_snapshot(&self) -> Result<(), String> {
        create_snapshot(&self.workspace_root, &self.memory)
    }

    pub fn delete_snapshot(&self) {
        delete_snapshot(&self.workspace_root)
    }

    pub fn save_pending_proposals(&self, batch: &MemoryProposalBatch) -> Result<(), String> {
        save_pending_proposals(&self.workspace_root, batch)
    }
}

// ── Snapshot & pending proposals ──

pub fn create_snapshot(workspace_root: &Path, memory: &AgentMemory) -> Result<(), String> {
    let dir = workspace_root.join(KNOWFORGE_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create .knowforge/memory dir: {e}"))?;
    let path = dir.join(SNAPSHOT_FILE);
    let tmp = dir.join(format!("{SNAPSHOT_FILE}.tmp"));
    let content = serde_json::to_string_pretty(memory)
        .map_err(|e| format!("Snapshot serialization failed: {e}"))?;
    std::fs::write(&tmp, format!("{content}\n"))
        .map_err(|e| format!("Snapshot write failed: {e}"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("Snapshot rename failed: {e}"))?;
    Ok(())
}

pub fn delete_snapshot(workspace_root: &Path) {
    let path = workspace_root.join(KNOWFORGE_DIR).join(SNAPSHOT_FILE);
    let _ = std::fs::remove_file(&path);
}

pub fn save_pending_proposals(
    workspace_root: &Path,
    batch: &MemoryProposalBatch,
) -> Result<(), String> {
    let dir = workspace_root.join(KNOWFORGE_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create .knowforge/memory dir: {e}"))?;
    let path = dir.join(PENDING_FILE);
    let tmp = dir.join(format!("{PENDING_FILE}.tmp"));
    let content = serde_json::to_string_pretty(batch)
        .map_err(|e| format!("Pending serialization failed: {e}"))?;
    std::fs::write(&tmp, format!("{content}\n"))
        .map_err(|e| format!("Pending write failed: {e}"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("Pending rename failed: {e}"))?;
    Ok(())
}

pub fn load_pending_proposals(workspace_root: &Path) -> Option<MemoryProposalBatch> {
    let path = workspace_root.join(KNOWFORGE_DIR).join(PENDING_FILE);
    let content = std::fs::read_to_string(&path).ok()?;
    let batch: MemoryProposalBatch = serde_json::from_str(&content).ok()?;

    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&batch.created_at) {
        let age_days = (Utc::now() - created.with_timezone(&Utc)).num_days();
        if age_days > extraction::PROPOSAL_EXPIRY_DAYS {
            let _ = std::fs::remove_file(&path);
            return None;
        }
    }

    Some(batch)
}

pub fn delete_pending_proposals(workspace_root: &Path) {
    let path = workspace_root.join(KNOWFORGE_DIR).join(PENDING_FILE);
    let _ = std::fs::remove_file(&path);
}

pub fn clear_memory_file(workspace_root: &Path) -> Result<(), String> {
    let path = workspace_root.join(KNOWFORGE_DIR).join(MEMORY_FILE);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to delete memory file: {e}"))?;
    }
    Ok(())
}
