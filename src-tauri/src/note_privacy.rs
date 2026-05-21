//! `kf-private` frontmatter 与相对路径校验（任务 09）。供 `llm` 在出站前统一裁决；供文件树展示锁标。

use serde_yaml::Value;
use std::io::Read;
use std::path::Path;

/// 从正文快照判断是否应按 **kf-private** 不透出全文（含解析失败时的安全优先策略）。
///
/// # 策略说明（与 `docs/ai_tasks/09-privacy-kf-private.md` §4 一致）
/// - 无标准 YAML frontmatter：返回 `false`（非私密）。
/// - 有起始 `---` 但缺少闭合 `---`：返回 `true`（不透出）。
/// - YAML 根非 mapping、或 `kf-private` 存在但类型非布尔：返回 `true`。
/// - `kf-private: true`：返回 `true`；缺键或 `false`：返回 `false`。
pub fn markdown_treat_as_kf_private(markdown: &str) -> bool {
    match split_frontmatter_yaml_owned(markdown) {
        Ok(None) => false,
        Err(()) => true,
        Ok(Some(yaml)) => {
            let trimmed = yaml.trim();
            if trimmed.is_empty() {
                return false;
            }
            match serde_yaml::from_str::<Value>(trimmed) {
                Ok(v) => is_kf_private_from_root_value(&v),
                Err(_) => true,
            }
        }
    }
}

/// `rel_path` 必须为 Vault 内相对路径（`/` 分隔），禁止逃逸。
/// 读取 Markdown 文件前缀（UTF-8 有损）并判定是否 `kf-private`；用于构建文件树，避免整文件读入。
/// 读取文件前 24KB 判断是否标记为 kf-private。
/// fail-closed：文件无法打开或读取时返回 `true`（视为私密），防止 I/O 错误导致私密内容泄露。
pub fn peek_kf_private_from_md_file(path: &Path) -> bool {
    const PREFIX: usize = 24_576;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return true,
    };
    let mut buf = vec![0u8; PREFIX];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return true,
    };
    buf.truncate(n);
    let head = String::from_utf8_lossy(&buf);
    markdown_treat_as_kf_private(head.as_ref())
}

pub fn validate_workspace_rel_path(rel_path: &str) -> Result<(), String> {
    let rel = rel_path.trim();
    if rel.is_empty() {
        return Err("Invalid note path.".to_string());
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err("Invalid note path.".to_string());
    }
    #[cfg(windows)]
    if rel.contains(':') {
        return Err("Invalid note path.".to_string());
    }
    if rel.contains('\\') {
        return Err("Invalid note path.".to_string());
    }
    for seg in rel.split('/') {
        if seg.is_empty() {
            return Err("Invalid note path.".to_string());
        }
        if seg == "." || seg == ".." {
            return Err("Invalid note path.".to_string());
        }
    }
    Ok(())
}

fn split_frontmatter_yaml_owned(src: &str) -> Result<Option<String>, ()> {
    let s = src.trim_start();
    if !s.starts_with("---") {
        return Ok(None);
    }
    let mut lines = s.lines();
    let first = lines.next().unwrap_or("");
    if first.trim().trim_end_matches('\r') != "---" {
        return Ok(None);
    }
    let mut yaml_lines: Vec<&str> = Vec::new();
    for line in lines {
        let t = line.trim().trim_end_matches('\r');
        if t == "---" {
            return Ok(Some(yaml_lines.join("\n")));
        }
        yaml_lines.push(line);
    }
    Err(())
}

fn is_kf_private_from_root_value(v: &Value) -> bool {
    let Some(m) = v.as_mapping() else {
        // 根不是 mapping：异常，安全优先
        return true;
    };
    let key = Value::String("kf-private".to_string());
    match m.get(&key) {
        None => false,
        Some(Value::Bool(b)) => *b,
        // 键存在但类型非布尔：不猜测，按私密处理
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter_not_private() {
        assert!(!markdown_treat_as_kf_private("# Hi\n"));
    }

    #[test]
    fn kf_private_true() {
        let md = "---\nkf-private: true\n---\n# x\n";
        assert!(markdown_treat_as_kf_private(md));
    }

    #[test]
    fn kf_private_false() {
        let md = "---\nkf-private: false\n---\n# x\n";
        assert!(!markdown_treat_as_kf_private(md));
    }

    #[test]
    fn unclosed_frontmatter_is_private() {
        let md = "---\nkf-private: false\n# no closing\n";
        assert!(markdown_treat_as_kf_private(md));
    }

    #[test]
    fn rel_path_rejects_dotdot() {
        assert!(validate_workspace_rel_path("a/../../etc").is_err());
    }

    #[test]
    fn peek_file_detects_kf_private() {
        let dir = std::env::temp_dir().join(format!(
            "kfpeek_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("p.md");
        std::fs::write(&p, "---\nkf-private: true\n---\n# x\n").unwrap();
        assert!(peek_kf_private_from_md_file(&p));
        let q = dir.join("q.md");
        std::fs::write(&q, "# no fm\n").unwrap();
        assert!(!peek_kf_private_from_md_file(&q));
    }
}
