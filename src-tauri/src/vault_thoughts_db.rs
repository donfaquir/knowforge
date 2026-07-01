//! Vault 侧车：`.knowforge/thoughts/index.sqlite` 存随手想法正文（SSOT）与查询索引。

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// 与 `thought_parser::USER_VERSION` 区分：侧车库独立迁移版本
pub const THOUGHTS_DB_USER_VERSION: i32 = 2;

/// 单条想法正文上限（Unicode 标量个数近似为字符数）
pub const MAX_THOUGHT_BODY_CHARS: usize = 131_072;

pub fn thoughts_db_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".knowforge/thoughts/index.sqlite")
}

/// 打开侧车库（不存在则创建），启用 WAL。
pub fn open_thoughts_db(vault_root: &Path) -> Result<Connection, String> {
    let path = thoughts_db_path(vault_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 .knowforge/thoughts 目录失败: {e}"))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开 thoughts SQLite 失败: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 失败: {e}"))?;
    init_schema(&conn)?;
    Ok(conn)
}

/// 若表上尚无 `standalone` 列则执行迁移（幂等）
fn migrate_v1_to_v2(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(thoughts)")
        .map_err(|e| format!("PRAGMA table_info 失败: {e}"))?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if cols.iter().any(|c| c == "standalone") {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE thoughts ADD COLUMN standalone INTEGER NOT NULL DEFAULT 0;",
    )
    .map_err(|e| format!("迁移 thoughts V2 失败: {e}"))?;
    Ok(())
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thoughts (
            thought_id TEXT PRIMARY KEY NOT NULL,
            note_stable_id TEXT NOT NULL,
            note_rel_path TEXT NOT NULL,
            body TEXT NOT NULL,
            summary TEXT,
            maturity TEXT NOT NULL,
            temporary INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            challenge_pass_count INTEGER NOT NULL DEFAULT 0,
            last_reviewed_at TEXT,
            standalone INTEGER NOT NULL DEFAULT 0,
            schema_version INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_thoughts_note_stable ON thoughts(note_stable_id);
        CREATE INDEX IF NOT EXISTS idx_thoughts_note_rel ON thoughts(note_rel_path);
        CREATE INDEX IF NOT EXISTS idx_thoughts_updated ON thoughts(updated_at);
        "#,
    )
    .map_err(|e| format!("初始化 thoughts 表失败: {e}"))?;

    migrate_v1_to_v2(conn)?;

    let ver: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    if ver < THOUGHTS_DB_USER_VERSION {
        conn.pragma_update(None, "user_version", THOUGHTS_DB_USER_VERSION)
            .map_err(|e| format!("设置 user_version 失败: {e}"))?;
    }
    Ok(())
}

/// 校验正文长度（Unicode 标量个数，与 `chars()` 一致）
///
/// UTF-8 下每个标量至少 1 字节，故 `body.len() <= MAX` 时标量数必不超上限（O(1) 常见路径）。
/// 仅当字节数已超上限时，全 ASCII 可直接按字节判错；含多字节时再逐标量计数，并在超过上限后立即返回，避免对整段超长文本做完整 `count()`。
pub fn validate_body_text(body: &str) -> Result<(), String> {
    if body.len() <= MAX_THOUGHT_BODY_CHARS {
        return Ok(());
    }
    if body.is_ascii() {
        return Err(format!(
            "想法正文过长（{} 字符），上限为 {MAX_THOUGHT_BODY_CHARS}",
            body.len()
        ));
    }
    let mut n = 0usize;
    for _ in body.chars() {
        n += 1;
        if n > MAX_THOUGHT_BODY_CHARS {
            return Err(format!(
                "想法正文过长（{n} 字符），上限为 {MAX_THOUGHT_BODY_CHARS}"
            ));
        }
    }
    Ok(())
}

pub fn upsert_thought_body(
    conn: &Connection,
    thought_id: &str,
    note_stable_id: &str,
    note_rel_path: &str,
    body: &str,
    summary: Option<&str>,
    maturity: &str,
    temporary: bool,
    standalone: bool,
    created_at: &str,
    updated_at: &str,
    challenge_pass_count: u32,
    last_reviewed_at: Option<&str>,
) -> Result<(), String> {
    validate_body_text(body)?;
    let temp_i = if temporary { 1 } else { 0 };
    let stand_i = if standalone { 1 } else { 0 };
    conn.execute(
        r#"INSERT INTO thoughts (
            thought_id, note_stable_id, note_rel_path, body, summary,
            maturity, temporary, standalone, created_at, updated_at,
            challenge_pass_count, last_reviewed_at, schema_version
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1)
        ON CONFLICT(thought_id) DO UPDATE SET
            note_stable_id = excluded.note_stable_id,
            note_rel_path = excluded.note_rel_path,
            body = excluded.body,
            summary = excluded.summary,
            maturity = excluded.maturity,
            temporary = excluded.temporary,
            standalone = excluded.standalone,
            updated_at = excluded.updated_at,
            challenge_pass_count = excluded.challenge_pass_count,
            last_reviewed_at = excluded.last_reviewed_at
        "#,
        params![
            thought_id,
            note_stable_id,
            note_rel_path,
            body,
            summary,
            maturity,
            temp_i,
            stand_i,
            created_at,
            updated_at,
            challenge_pass_count,
            last_reviewed_at,
        ],
    )
    .map_err(|e| format!("写入 thought 失败: {e}"))?;
    Ok(())
}

/// 读取正文；无行则 None
pub fn get_body(conn: &Connection, thought_id: &str) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT body FROM thoughts WHERE thought_id = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![thought_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    if let Some(r) = rows.next() {
        let body: String = r.map_err(|e| e.to_string())?;
        return Ok(Some(body));
    }
    Ok(None)
}

/// 按 `note_stable_id` 刷新缓存路径（不更新独立想法行）
pub fn refresh_rel_path_for_stable_id(
    conn: &Connection,
    note_stable_id: &str,
    new_rel_path: &str,
) -> Result<usize, String> {
    let n = conn
        .execute(
            "UPDATE thoughts SET note_rel_path = ?2 WHERE note_stable_id = ?1 AND standalone = 0",
            params![note_stable_id, new_rel_path],
        )
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// 更新成熟度及挑战相关字段（与 YAML 写回一致）
pub fn update_thought_after_challenge(
    conn: &Connection,
    thought_id: &str,
    maturity: &str,
    updated_at: &str,
    challenge_pass_count: u32,
    last_reviewed_at: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        r#"UPDATE thoughts SET
            maturity = ?2,
            updated_at = ?3,
            challenge_pass_count = ?4,
            last_reviewed_at = ?5
        WHERE thought_id = ?1"#,
        params![
            thought_id,
            maturity,
            updated_at,
            challenge_pass_count,
            last_reviewed_at,
        ],
    )
    .map_err(|e| format!("更新 thought 成熟度失败: {e}"))?;
    Ok(())
}

/// 想法图：每笔记条数 + 最高成熟度序（0=seedling,2=mature）；排除独立想法虚拟路径
pub fn graph_thought_stats(conn: &Connection) -> Result<Vec<(String, usize, u8)>, String> {
    let mut stmt = conn
        .prepare(
            r#"SELECT note_rel_path,
                      COUNT(*) AS c,
                      MAX(CASE maturity WHEN 'mature' THEN 2 WHEN 'growing' THEN 1 ELSE 0 END) AS mx
               FROM thoughts
               WHERE standalone = 0
               GROUP BY note_rel_path"#,
        )
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as usize,
                row.get::<_, i64>(2)? as u8,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// 回顾排期：侧车行 + 元数据列（YAML 不再扫 callout）；不含独立想法
pub fn list_thought_rows_for_review(
    conn: &Connection,
) -> Result<
    Vec<(
        String,
        String,
        String,
        String,
        bool,
        String,
        String,
        i64,
        Option<String>,
    )>,
    String,
> {
    let mut stmt = conn
        .prepare(
            "SELECT note_rel_path, thought_id, body, maturity, temporary, created_at, updated_at, challenge_pass_count, last_reviewed_at FROM thoughts WHERE standalone = 0",
        )
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// 检索：返回 (rel_path, thought_id, body, maturity_str) 供上层打分
pub fn list_all_thought_rows_for_scan(
    conn: &Connection,
) -> Result<Vec<(String, String, String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT note_rel_path, thought_id, body, maturity FROM thoughts")
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// 全库想法列表行（供 IPC / UI）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultThoughtListRow {
    pub rel_path: String,
    pub thought_id: String,
    pub excerpt: String,
    pub maturity: String,
    pub temporary: bool,
    pub standalone: bool,
    pub updated_at: String,
}

/// IPC / 详情：单条想法完整字段
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtDetail {
    pub thought_id: String,
    pub note_stable_id: String,
    pub note_rel_path: String,
    pub body: String,
    pub summary: Option<String>,
    pub maturity: String,
    pub temporary: bool,
    pub standalone: bool,
    pub created_at: String,
    pub updated_at: String,
    pub challenge_pass_count: u32,
    pub last_reviewed_at: Option<String>,
}

fn excerpt_from_body(body: &str, max_chars: usize) -> String {
    let t = body.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    let mut end = max_chars;
    while !t.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &t[..end])
}

// list_vault_thought_rows 共用 SELECT 列表，避免与 WHERE 分支拼接时漂移
const LIST_VAULT_THOUGHT_ROWS_SELECT: &str =
    "SELECT note_rel_path, thought_id, body, maturity, temporary, standalone, updated_at FROM thoughts";

/// `all` | `standalone` | `linked` | `temporary`；非法值视为 `all`
fn filter_sql_clause(filter: Option<&str>) -> &'static str {
    match filter.map(str::trim).filter(|s| !s.is_empty()) {
        Some("standalone") => " AND standalone = 1 ",
        Some("linked") => " AND standalone = 0 ",
        Some("temporary") => " AND temporary = 1 ",
        _ => " ",
    }
}

/// 分页列表响应（IPC / UI）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultThoughtListPage {
    pub rows: Vec<VaultThoughtListRow>,
    pub total: usize,
}

/// 按更新时间倒序列出想法（分页）。`query` 非空时在正文、相对路径、thought_id 上做 ASCII 大小写不敏感子串匹配。
pub fn list_vault_thought_rows_paged(
    conn: &Connection,
    limit: usize,
    offset: usize,
    query: Option<&str>,
    filter: Option<&str>,
) -> Result<VaultThoughtListPage, String> {
    let cap = limit.clamp(1, 2000) as i64;
    let off = offset.min(1_000_000_000) as i64;
    let needle = query
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let filt = filter_sql_clause(filter);

    let map_row =
        |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, String, String, bool, i64, String)> {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, i64>(4)? != 0,
                row.get::<_, i64>(5)?,
                row.get(6)?,
            ))
        };

    let total: i64 = if let Some(ref n) = needle {
        let count_sql = format!(
            "SELECT COUNT(*) FROM thoughts WHERE (instr(lower(body), ?1) > 0 \
             OR instr(lower(note_rel_path), ?1) > 0 OR instr(lower(thought_id), ?1) > 0){filt}"
        );
        conn.query_row(count_sql.as_str(), params![n.as_str()], |row| row.get(0))
            .map_err(|e| e.to_string())?
    } else {
        let count_sql = format!("SELECT COUNT(*) FROM thoughts WHERE 1=1{filt}");
        conn.query_row(count_sql.as_str(), [], |row| row.get(0))
            .map_err(|e| e.to_string())?
    };

    let rows: Vec<(String, String, String, String, bool, i64, String)> = if let Some(ref n) = needle {
        let sql = format!(
            "{LIST_VAULT_THOUGHT_ROWS_SELECT} WHERE (instr(lower(body), ?1) > 0 \
             OR instr(lower(note_rel_path), ?1) > 0 OR instr(lower(thought_id), ?1) > 0){filt}\
             ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(params![n.as_str(), cap, off], map_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    } else {
        let sql = format!(
            "{LIST_VAULT_THOUGHT_ROWS_SELECT} WHERE 1=1{filt} ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(params![cap, off], map_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    let mut out = Vec::with_capacity(rows.len());
    for (rel_path, thought_id, body, maturity, temporary, stand_i, updated_at) in rows {
        out.push(VaultThoughtListRow {
            rel_path,
            thought_id,
            excerpt: excerpt_from_body(&body, 200),
            maturity,
            temporary,
            standalone: stand_i != 0,
            updated_at,
        });
    }
    Ok(VaultThoughtListPage {
        rows: out,
        total: total.max(0) as usize,
    })
}

/// 新建独立想法：仅 SQLite，返回 `thought_id`
pub fn create_standalone_thought(
    conn: &Connection,
    body: &str,
    summary: Option<&str>,
) -> Result<String, String> {
    validate_body_text(body)?;
    let thought_id = format!("thought-{}", uuid::Uuid::new_v4().simple());
    let note_stable_id = thought_id.clone();
    let note_rel_path = format!(".knowforge/standalone/{thought_id}");
    let now = Utc::now().to_rfc3339();
    upsert_thought_body(
        conn,
        &thought_id,
        &note_stable_id,
        &note_rel_path,
        body,
        summary,
        "seedling",
        false,
        true,
        &now,
        &now,
        0,
        None,
    )?;
    Ok(thought_id)
}

/// 按 id 更新正文与摘要（不改成熟度等，除非后续扩展）
pub fn update_thought_body_by_id(
    conn: &Connection,
    thought_id: &str,
    new_body: &str,
    new_summary: Option<&str>,
) -> Result<(), String> {
    validate_body_text(new_body)?;
    let now = Utc::now().to_rfc3339();
    let n = conn
        .execute(
            r#"UPDATE thoughts SET body = ?2, summary = ?3, updated_at = ?4
               WHERE thought_id = ?1"#,
            params![thought_id, new_body, new_summary, now],
        )
        .map_err(|e| format!("更新 thought 正文失败: {e}"))?;
    if n == 0 {
        return Err("找不到该 thought_id".to_string());
    }
    Ok(())
}

/// 删除想法行；返回是否实际删除
pub fn delete_thought_by_id(conn: &Connection, thought_id: &str) -> Result<bool, String> {
    let n = conn
        .execute(
            "DELETE FROM thoughts WHERE thought_id = ?1",
            params![thought_id],
        )
        .map_err(|e| format!("删除 thought 失败: {e}"))?;
    Ok(n > 0)
}

/// 单条详情；无行返回 None
pub fn get_thought_detail(conn: &Connection, thought_id: &str) -> Result<Option<ThoughtDetail>, String> {
    let mut stmt = conn
        .prepare(
            r#"SELECT thought_id, note_stable_id, note_rel_path, body, summary, maturity,
                      temporary, standalone, created_at, updated_at, challenge_pass_count, last_reviewed_at
               FROM thoughts WHERE thought_id = ?1"#,
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![thought_id], |row| {
            Ok(ThoughtDetail {
                thought_id: row.get(0)?,
                note_stable_id: row.get(1)?,
                note_rel_path: row.get(2)?,
                body: row.get(3)?,
                summary: row.get(4)?,
                maturity: row.get(5)?,
                temporary: row.get::<_, i64>(6)? != 0,
                standalone: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                challenge_pass_count: row.get::<_, i64>(10)? as u32,
                last_reviewed_at: row.get(11)?,
            })
        })
        .map_err(|e| e.to_string())?;
    if let Some(r) = rows.next() {
        return Ok(Some(r.map_err(|e| e.to_string())?));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn get_thought_detail_returns_full_metadata() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let conn = open_thoughts_db(root).unwrap();

        upsert_thought_body(
            &conn,
            "t-001",
            "note-abc",
            "notes/rust.md",
            "Ownership in Rust prevents data races at compile time.",
            Some("Rust ownership"),
            "budding",
            false,
            false,
            "2026-06-01T10:00:00Z",
            "2026-06-15T12:00:00Z",
            3,
            Some("2026-06-14T09:00:00Z"),
        )
        .unwrap();

        let detail = get_thought_detail(&conn, "t-001").unwrap().unwrap();
        assert_eq!(detail.thought_id, "t-001");
        assert_eq!(detail.note_stable_id, "note-abc");
        assert_eq!(detail.note_rel_path, "notes/rust.md");
        assert_eq!(detail.body, "Ownership in Rust prevents data races at compile time.");
        assert_eq!(detail.summary.as_deref(), Some("Rust ownership"));
        assert_eq!(detail.maturity, "budding");
        assert!(!detail.temporary);
        assert!(!detail.standalone);
        assert_eq!(detail.created_at, "2026-06-01T10:00:00Z");
        assert_eq!(detail.updated_at, "2026-06-15T12:00:00Z");
        assert_eq!(detail.challenge_pass_count, 3);
        assert_eq!(detail.last_reviewed_at.as_deref(), Some("2026-06-14T09:00:00Z"));
    }

    #[test]
    fn get_thought_detail_returns_none_for_missing() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let conn = open_thoughts_db(root).unwrap();
        assert!(get_thought_detail(&conn, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn migrate_idempotent() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let path = thoughts_db_path(root);
        fs::create_dir_all(path.parent().expect("parent")).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"CREATE TABLE thoughts (
                thought_id TEXT PRIMARY KEY NOT NULL,
                note_stable_id TEXT NOT NULL,
                note_rel_path TEXT NOT NULL,
                body TEXT NOT NULL,
                summary TEXT,
                maturity TEXT NOT NULL,
                temporary INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                challenge_pass_count INTEGER NOT NULL DEFAULT 0,
                last_reviewed_at TEXT,
                schema_version INTEGER NOT NULL DEFAULT 1
            );"#,
            )
            .unwrap();
        }
        let _conn = open_thoughts_db(root).unwrap();
        let conn = Connection::open(&path).unwrap();
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, THOUGHTS_DB_USER_VERSION);
        let n: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('thoughts') WHERE name='standalone'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
