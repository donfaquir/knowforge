//! 根据各笔记 YAML 中的 `kfVaultNoteId` 刷新侧车表中的 `note_rel_path` 缓存（外部改名 / 移动后对账）。
//! 若两文件声明同一 `kfVaultNoteId` 且相对路径不同，整次对账 `Err` 并**不写库**，避免静默覆盖。

use crate::thought_parser::{parse_kf_vault_note_id, split_frontmatter, FrontmatterSplit};
use crate::vault_context_search;
use crate::vault_thoughts_db;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const SCAN_BYTES: usize = 256 * 1024;
const MAX_FILES: usize = 12_000;

/// 全库扫描：建立 `kfVaultNoteId` → 当前 `rel_path` 映射并批量 `UPDATE thoughts`。
pub fn reconcile_thought_rel_paths_blocking(root: &Path) -> Result<(), String> {
    let conn = vault_thoughts_db::open_thoughts_db(root)?;
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(root, root, &mut paths, MAX_FILES)?;

    let mut stable_to_rel: HashMap<String, String> = HashMap::new();

    for abs in paths {
        let Some(rel) = vault_context_search::rel_path_from_root(root, &abs) else {
            continue;
        };
        let bytes = match fs::read(&abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > SCAN_BYTES {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let yaml = match split_frontmatter(&text) {
            FrontmatterSplit::Closed { yaml, .. } => yaml,
            _ => continue,
        };
        let Some(stable) = parse_kf_vault_note_id(&yaml) else {
            continue;
        };
        if stable.is_empty() {
            continue;
        }
        // 同一 stable id 只能对应一条笔记路径；否则后写入库会覆盖前者，导致引用丢失
        match stable_to_rel.entry(stable.clone()) {
            Entry::Vacant(v) => {
                v.insert(rel);
            }
            Entry::Occupied(o) => {
                if o.get() != &rel {
                    return Err(format!(
                        "duplicate kfVaultNoteId {:?}: first {:?}, also {:?} — each id must be unique in frontmatter",
                        stable,
                        o.get(),
                        rel
                    ));
                }
            }
        }
    }

    for (stable_id, rel_path) in stable_to_rel {
        vault_thoughts_db::refresh_rel_path_for_stable_id(&conn, &stable_id, &rel_path)?;
    }
    Ok(())
}

/// 单文件移动后：按新路径读 YAML 并刷新该笔记在侧车中的路径缓存。
pub fn refresh_note_rel_path_after_file_move(root: &Path, new_rel_path: &str) -> Result<(), String> {
    let joined = crate::join_under_root(root, new_rel_path)?;
    let text = match fs::read_to_string(&joined) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let yaml = match split_frontmatter(&text) {
        FrontmatterSplit::Closed { yaml, .. } => yaml,
        _ => return Ok(()),
    };
    let Some(stable) = parse_kf_vault_note_id(&yaml) else {
        return Ok(());
    };
    if stable.is_empty() {
        return Ok(());
    }
    let conn = vault_thoughts_db::open_thoughts_db(root)?;
    vault_thoughts_db::refresh_rel_path_for_stable_id(&conn, &stable, new_rel_path)?;
    Ok(())
}
