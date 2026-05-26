//! 工作区写操作的统一路径安全校验。
//!
//! 三层防护:
//! 1. 字符串层 — 复用 `note_privacy::validate_workspace_rel_path` 拒绝 `..`、绝对路径、Windows 盘符等。
//! 2. canonicalize 解析 — 实际文件 / 父目录均 canonicalize,避免符号链接逃逸。
//! 3. starts_with — canonical 结果必须仍在 workspace_root 之下。
//!
//! `workspace_root` 应当是调用方已经 canonicalize 过的路径。

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum PathSafetyError {
    /// 字符串层校验失败(`..`、绝对路径、非法字符等)。
    InvalidRelPath(String),
    /// 文件不存在(仅 `resolve_existing_under_root` 触发)。
    NotFound(String),
    /// canonicalize 后路径不在 workspace_root 之下(常见原因:符号链接逃逸)。
    OutsideWorkspace,
    /// I/O 错误,已脱敏。
    Io(String),
}

impl std::fmt::Display for PathSafetyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRelPath(m) => write!(f, "invalid rel_path: {m}"),
            Self::NotFound(p) => write!(f, "not found: {p}"),
            Self::OutsideWorkspace => write!(f, "path escapes workspace root"),
            Self::Io(m) => write!(f, "{m}"),
        }
    }
}

/// 解析一个**已存在**的文件路径。
/// 返回 canonicalize 后的绝对路径,保证位于 `workspace_root` 之下。
pub fn resolve_existing_under_root(
    workspace_root: &Path,
    rel_path: &str,
) -> Result<PathBuf, PathSafetyError> {
    crate::note_privacy::validate_workspace_rel_path(rel_path)
        .map_err(PathSafetyError::InvalidRelPath)?;

    let joined = workspace_root.join(rel_path);
    if !joined.exists() {
        return Err(PathSafetyError::NotFound(rel_path.to_string()));
    }

    let canonical = std::fs::canonicalize(&joined)
        .map_err(|e| PathSafetyError::Io(format!("resolve path: {e}")))?;

    if !canonical.starts_with(workspace_root) {
        return Err(PathSafetyError::OutsideWorkspace);
    }
    Ok(canonical)
}

/// 解析一个**尚未创建**的文件路径。
/// 因目标不存在无法 canonicalize 自身,改为 canonicalize **第一个已存在的祖先**,
/// 然后用已校验为安全的 rel_path 尾段重组。
pub fn resolve_new_under_root(
    workspace_root: &Path,
    rel_path: &str,
) -> Result<PathBuf, PathSafetyError> {
    crate::note_privacy::validate_workspace_rel_path(rel_path)
        .map_err(PathSafetyError::InvalidRelPath)?;

    let joined = workspace_root.join(rel_path);

    // 找到 joined 链上第一个已存在的祖先,并记录从该祖先到 joined 之间还缺多少层。
    let mut ancestor = joined.as_path();
    let mut missing_depth: usize = 0;
    let canonical_ancestor = loop {
        if ancestor.exists() {
            break std::fs::canonicalize(ancestor)
                .map_err(|e| PathSafetyError::Io(format!("resolve ancestor: {e}")))?;
        }
        match ancestor.parent() {
            Some(p) => {
                ancestor = p;
                missing_depth += 1;
            }
            None => return Err(PathSafetyError::OutsideWorkspace),
        }
    };

    if !canonical_ancestor.starts_with(workspace_root) {
        return Err(PathSafetyError::OutsideWorkspace);
    }

    // 取 joined 末尾 missing_depth 层,叠到 canonical_ancestor 之上。
    // (rel_path 已通过字符串校验,不含 `..`,这次拼接不会向上越界。)
    let tail_components: Vec<_> = joined
        .components()
        .rev()
        .take(missing_depth)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut out = canonical_ancestor;
    for comp in tail_components {
        out.push(comp.as_os_str());
    }

    if !out.starts_with(workspace_root) {
        return Err(PathSafetyError::OutsideWorkspace);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "kf-path-safety-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&p).unwrap();
        fs::canonicalize(&p).unwrap()
    }

    #[test]
    fn existing_rejects_traversal() {
        let root = tempdir();
        let err = resolve_existing_under_root(&root, "../escape.md").unwrap_err();
        assert!(matches!(err, PathSafetyError::InvalidRelPath(_)));
    }

    #[test]
    fn existing_rejects_absolute() {
        let root = tempdir();
        let err = resolve_existing_under_root(&root, "/etc/passwd").unwrap_err();
        assert!(matches!(err, PathSafetyError::InvalidRelPath(_)));
    }

    #[test]
    fn existing_reports_not_found() {
        let root = tempdir();
        let err = resolve_existing_under_root(&root, "missing.md").unwrap_err();
        assert!(matches!(err, PathSafetyError::NotFound(_)));
    }

    #[test]
    fn existing_resolves_real_file() {
        let root = tempdir();
        fs::write(root.join("a.md"), "x").unwrap();
        let resolved = resolve_existing_under_root(&root, "a.md").unwrap();
        assert!(resolved.starts_with(&root));
        assert!(resolved.ends_with("a.md"));
    }

    #[cfg(unix)]
    #[test]
    fn existing_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let root = tempdir();
        let outside = tempdir();
        let secret = outside.join("secret.md");
        fs::write(&secret, "leak").unwrap();
        symlink(&secret, root.join("link.md")).unwrap();
        let err = resolve_existing_under_root(&root, "link.md").unwrap_err();
        assert!(matches!(err, PathSafetyError::OutsideWorkspace));
    }

    #[test]
    fn new_rejects_traversal() {
        let root = tempdir();
        let err = resolve_new_under_root(&root, "../escape.md").unwrap_err();
        assert!(matches!(err, PathSafetyError::InvalidRelPath(_)));
    }

    #[test]
    fn new_in_existing_dir() {
        let root = tempdir();
        let resolved = resolve_new_under_root(&root, "subdir/new.md").unwrap();
        assert!(resolved.starts_with(&root));
        assert!(resolved.ends_with("new.md"));
    }

    #[cfg(unix)]
    #[test]
    fn new_parent_symlink_escape_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempdir();
        let outside = tempdir();
        symlink(&outside, root.join("evil")).unwrap();
        let err = resolve_new_under_root(&root, "evil/x.md").unwrap_err();
        assert!(matches!(err, PathSafetyError::OutsideWorkspace));
    }
}
