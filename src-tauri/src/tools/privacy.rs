use std::path::Path;

use super::context::PrivacyFilter;

// ─── KfPrivateFilter ───────────────────────────────────────────────────────────
// 基于现有 note_privacy 模块的隐私过滤器

pub struct KfPrivateFilter;

impl PrivacyFilter for KfPrivateFilter {
    fn filter_note_content(&self, content: &str) -> (String, u32) {
        if crate::note_privacy::markdown_treat_as_kf_private(content) {
            ("[整篇笔记已隐藏]".to_string(), 1)
        } else {
            (content.to_string(), 0)
        }
    }

    fn is_private_path(&self, rel_path: &str, workspace_root: &Path) -> bool {
        let full_path = workspace_root.join(rel_path);
        crate::note_privacy::peek_kf_private_from_md_file(&full_path)
    }
}
