use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::thought_parser::{split_frontmatter, FrontmatterSplit};
use crate::topic_network;

use super::types::*;

pub(super) const WORKSPACE_STALENESS_DAYS: i64 = 7;

const SAMPLE_FILE_COUNT: usize = 30;
const CONTENT_PEEK_CHARS: usize = 1024;
const MAX_TAG_VOCABULARY: usize = 30;
pub(super) const MAX_FREQUENT_PATHS: usize = 15;
const MAX_TOPICS: usize = 20;
pub(super) fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{AC00}'..='\u{D7AF}'
    )
}

pub(super) fn evenly_spaced_indices(total: usize, max_sample: usize) -> Vec<usize> {
    if total == 0 {
        return Vec::new();
    }
    if total <= max_sample {
        return (0..total).collect();
    }
    let step = total as f64 / max_sample as f64;
    (0..max_sample).map(|i| (i as f64 * step) as usize).collect()
}

pub(super) fn extract_yaml_tags(yaml: &str, out: &mut HashSet<String>) {
    let mut in_tags = false;
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tags:") {
            let rest = trimmed.strip_prefix("tags:").unwrap().trim();
            if rest.starts_with('[') {
                let inner = rest.trim_start_matches('[').trim_end_matches(']');
                for tag in inner.split(',') {
                    let tag = tag.trim().trim_matches('"').trim_matches('\'').trim();
                    if !tag.is_empty() {
                        out.insert(tag.to_string());
                    }
                }
                return;
            }
            in_tags = true;
            continue;
        }
        if in_tags {
            if trimmed.starts_with("- ") {
                let tag = trimmed
                    .strip_prefix("- ")
                    .unwrap()
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !tag.is_empty() {
                    out.insert(tag.to_string());
                }
            } else if !trimmed.is_empty() {
                in_tags = false;
            }
        }
    }
}

pub fn observe_workspace(workspace_root: &Path, note_paths: &[String]) -> MemoryWorkspace {
    if note_paths.is_empty() {
        return MemoryWorkspace {
            updated_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        };
    }

    let sample_indices = evenly_spaced_indices(note_paths.len(), SAMPLE_FILE_COUNT);

    // 1. Language distribution + tag extraction in a single pass over sampled files
    let mut cjk_count: usize = 0;
    let mut ascii_count: usize = 0;
    let mut tag_set: HashSet<String> = HashSet::new();

    for &idx in &sample_indices {
        let full_path = workspace_root.join(&note_paths[idx]);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            for c in content.chars().take(CONTENT_PEEK_CHARS) {
                if is_cjk(c) {
                    cjk_count += 1;
                } else if c.is_ascii_alphanumeric() {
                    ascii_count += 1;
                }
            }
            if let FrontmatterSplit::Closed { yaml, .. } = split_frontmatter(&content) {
                extract_yaml_tags(&yaml, &mut tag_set);
            }
        }
    }

    // Fallback to filename-based counting when no content was readable
    if cjk_count == 0 && ascii_count == 0 {
        for path in note_paths {
            let filename = path.rsplit('/').next().unwrap_or(path);
            let stem = filename.strip_suffix(".md").unwrap_or(filename);
            for c in stem.chars() {
                if is_cjk(c) {
                    cjk_count += 1;
                } else if c.is_ascii_alphanumeric() {
                    ascii_count += 1;
                }
            }
        }
    }

    let total_chars = (cjk_count + ascii_count).max(1);
    let mut language_distribution = HashMap::new();
    let zh_ratio = cjk_count as f64 / total_chars as f64;
    let en_ratio = ascii_count as f64 / total_chars as f64;
    if zh_ratio > 0.01 {
        language_distribution.insert("zh".to_string(), (zh_ratio * 100.0).round() / 100.0);
    }
    if en_ratio > 0.01 {
        language_distribution.insert("en".to_string(), (en_ratio * 100.0).round() / 100.0);
    }

    // 2. Frequent paths: count notes per parent directory
    let mut dir_counts: HashMap<String, usize> = HashMap::new();
    for path in note_paths {
        let parent = match path.rfind('/') {
            Some(idx) => &path[..idx + 1],
            None => "",
        };
        if !parent.is_empty() {
            *dir_counts.entry(parent.to_string()).or_default() += 1;
        }
    }
    let mut dir_pairs: Vec<(String, usize)> = dir_counts.into_iter().collect();
    dir_pairs.sort_by(|a, b| b.1.cmp(&a.1));
    dir_pairs.truncate(MAX_FREQUENT_PATHS);
    let frequent_paths: Vec<FrequentPath> = dir_pairs
        .into_iter()
        .map(|(path, count)| FrequentPath {
            path: path.clone(),
            description: format!("{count} notes"),
        })
        .collect();

    // 3. Tag vocabulary from frontmatter
    let mut tag_vocabulary: Vec<String> = tag_set.into_iter().collect();
    tag_vocabulary.sort();
    tag_vocabulary.truncate(MAX_TAG_VOCABULARY);

    // 4. Topics: prefer canonical topics from SQLite, fallback to directory names
    let mut topics: Vec<String> = Vec::new();
    let db_path = topic_network::topic_db_path(workspace_root);
    if db_path.exists() {
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            if let Ok(canonicals) = topic_network::list_dictionary_canonicals(&conn) {
                topics = canonicals;
            }
        }
    }
    if topics.is_empty() {
        topics = frequent_paths
            .iter()
            .take(MAX_TOPICS)
            .filter_map(|fp| {
                let name = fp.path.trim_end_matches('/');
                let name = name.rsplit('/').next().unwrap_or(name);
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .collect();
    }
    topics.truncate(MAX_TOPICS);

    MemoryWorkspace {
        updated_at: Some(Utc::now().to_rfc3339()),
        language_distribution,
        frequent_paths,
        tag_vocabulary,
        topics,
    }
}

pub(super) fn is_workspace_stale(updated_at: &Option<String>) -> bool {
    let ts = match updated_at {
        Some(s) => s,
        None => return true,
    };
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => (Utc::now() - dt.with_timezone(&Utc)).num_days() >= WORKSPACE_STALENESS_DAYS,
        Err(_) => true,
    }
}

pub(super) fn scan_md_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    scan_md_recursive(root, root, &mut paths);
    paths
}

fn scan_md_recursive(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan_md_recursive(root, &path, out);
        } else if meta.is_file() && name_str.ends_with(".md") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}
