use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Serialize;

pub fn init_feedback_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS challenge_feedback (
            id TEXT PRIMARY KEY,
            thought_id TEXT,
            question_text TEXT NOT NULL,
            question_template TEXT,
            rating TEXT NOT NULL,
            rating_reason TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_cf_rating ON challenge_feedback(rating);
        CREATE INDEX IF NOT EXISTS idx_cf_template ON challenge_feedback(question_template);
        "#,
    )
    .map_err(|e| format!("init challenge_feedback table: {e}"))?;
    Ok(())
}

pub fn insert_feedback(
    conn: &Connection,
    thought_id: Option<&str>,
    question_text: &str,
    question_template: Option<&str>,
    rating: &str,
    rating_reason: Option<&str>,
) -> Result<(), String> {
    let id = format!("cf-{}", uuid::Uuid::new_v4());
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO challenge_feedback (id, thought_id, question_text, question_template, rating, rating_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, thought_id, question_text, question_template, rating, rating_reason, now],
    )
    .map_err(|e| format!("insert challenge feedback: {e}"))?;
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TemplateStats {
    pub template: String,
    pub total: usize,
    pub helpful: usize,
    pub not_helpful: usize,
    pub helpful_rate: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IssueCount {
    pub reason: String,
    pub count: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackStats {
    pub total_ratings: usize,
    pub helpful_count: usize,
    pub not_helpful_count: usize,
    pub helpful_rate: f64,
    pub by_template: Vec<TemplateStats>,
    pub common_issues: Vec<IssueCount>,
}

pub fn query_feedback_stats(conn: &Connection) -> Result<FeedbackStats, String> {
    let helpful_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM challenge_feedback WHERE rating = 'helpful'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let not_helpful_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM challenge_feedback WHERE rating = 'not_helpful'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let total_ratings = helpful_count + not_helpful_count;
    let helpful_rate = if total_ratings > 0 {
        helpful_count as f64 / total_ratings as f64
    } else {
        0.0
    };

    let mut stmt = conn
        .prepare(
            "SELECT question_template, rating, COUNT(*) as cnt
             FROM challenge_feedback
             WHERE question_template IS NOT NULL
             GROUP BY question_template, rating
             ORDER BY question_template",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, usize)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, usize>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut template_map: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    for (tmpl, rating, cnt) in &rows {
        let entry = template_map.entry(tmpl.clone()).or_insert((0, 0));
        match rating.as_str() {
            "helpful" => entry.0 += cnt,
            "not_helpful" => entry.1 += cnt,
            _ => {}
        }
    }
    let mut by_template: Vec<TemplateStats> = template_map
        .into_iter()
        .map(|(template, (h, nh))| {
            let total = h + nh;
            TemplateStats {
                template,
                total,
                helpful: h,
                not_helpful: nh,
                helpful_rate: if total > 0 { h as f64 / total as f64 } else { 0.0 },
            }
        })
        .collect();
    by_template.sort_by(|a, b| a.template.cmp(&b.template));

    let mut issue_stmt = conn
        .prepare(
            "SELECT rating_reason, COUNT(*) as cnt
             FROM challenge_feedback
             WHERE rating_reason IS NOT NULL AND rating_reason != ''
             GROUP BY rating_reason
             ORDER BY cnt DESC",
        )
        .map_err(|e| e.to_string())?;
    let common_issues: Vec<IssueCount> = issue_stmt
        .query_map([], |row| {
            Ok(IssueCount {
                reason: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(FeedbackStats {
        total_ratings,
        helpful_count,
        not_helpful_count,
        helpful_rate,
        by_template,
        common_issues,
    })
}

#[tauri::command]
pub async fn submit_challenge_feedback(
    state: tauri::State<'_, crate::WorkspaceState>,
    thought_id: Option<String>,
    question_text: String,
    question_template: Option<String>,
    rating: String,
    rating_reason: Option<String>,
) -> Result<(), String> {
    let root = crate::lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let conn = crate::vault_thoughts_db::open_thoughts_db(&root)?;
        insert_feedback(
            &conn,
            thought_id.as_deref(),
            &question_text,
            question_template.as_deref(),
            &rating,
            rating_reason.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_feedback_stats(
    state: tauri::State<'_, crate::WorkspaceState>,
) -> Result<FeedbackStats, String> {
    let root = crate::lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let conn = crate::vault_thoughts_db::open_thoughts_db(&root)?;
        query_feedback_stats(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_feedback_table(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_query_stats() {
        let conn = setup_db();
        insert_feedback(&conn, Some("t1"), "What is X?", Some("compare"), "helpful", None).unwrap();
        insert_feedback(&conn, Some("t2"), "Explain Y", Some("apply"), "not_helpful", Some("too_easy")).unwrap();
        insert_feedback(&conn, Some("t3"), "Compare A", Some("compare"), "helpful", None).unwrap();

        let stats = query_feedback_stats(&conn).unwrap();
        assert_eq!(stats.total_ratings, 3);
        assert_eq!(stats.helpful_count, 2);
        assert_eq!(stats.not_helpful_count, 1);
        assert!((stats.helpful_rate - 2.0 / 3.0).abs() < 0.01);

        let compare = stats.by_template.iter().find(|t| t.template == "compare").unwrap();
        assert_eq!(compare.helpful, 2);
        assert_eq!(compare.not_helpful, 0);

        let apply = stats.by_template.iter().find(|t| t.template == "apply").unwrap();
        assert_eq!(apply.helpful, 0);
        assert_eq!(apply.not_helpful, 1);

        assert_eq!(stats.common_issues.len(), 1);
        assert_eq!(stats.common_issues[0].reason, "too_easy");
        assert_eq!(stats.common_issues[0].count, 1);
    }

    #[test]
    fn empty_stats() {
        let conn = setup_db();
        let stats = query_feedback_stats(&conn).unwrap();
        assert_eq!(stats.total_ratings, 0);
        assert_eq!(stats.helpful_rate, 0.0);
        assert!(stats.by_template.is_empty());
        assert!(stats.common_issues.is_empty());
    }

    #[test]
    fn multiple_reasons() {
        let conn = setup_db();
        insert_feedback(&conn, None, "Q1", None, "not_helpful", Some("too_vague")).unwrap();
        insert_feedback(&conn, None, "Q2", None, "not_helpful", Some("too_vague")).unwrap();
        insert_feedback(&conn, None, "Q3", None, "not_helpful", Some("irrelevant")).unwrap();

        let stats = query_feedback_stats(&conn).unwrap();
        assert_eq!(stats.common_issues[0].reason, "too_vague");
        assert_eq!(stats.common_issues[0].count, 2);
        assert_eq!(stats.common_issues[1].reason, "irrelevant");
        assert_eq!(stats.common_issues[1].count, 1);
    }
}
