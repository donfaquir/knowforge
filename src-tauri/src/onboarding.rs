use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::vault_thoughts_db;

const SEED_MARKER: &str = ".knowforge/onboarding_seeded";

struct SeedThought {
    body: &'static str,
    summary: &'static str,
}

const SEED_THOUGHTS: &[SeedThought] = &[
    SeedThought {
        body: "费曼学习法的核心不是「简化」——是「发现你无法简化的地方」。当你无法用简单语言解释一个概念时，说明你的理解有漏洞。\n\nThe core of the Feynman technique isn't 'simplifying' — it's 'discovering what you can't simplify.' When you can't explain a concept in plain language, it reveals gaps in your understanding.",
        summary: "Feynman technique: expose understanding gaps",
    },
    SeedThought {
        body: "间隔效应说明大脑在「快要忘记」的时刻复习效果最好。这解释了为什么考前突击虽然能通过考试，但两周后什么都不记得。\n\nThe spacing effect shows that the brain learns best when reviewing at the moment of near-forgetting. This explains why cramming might pass the test, but leaves nothing two weeks later.",
        summary: "Spaced repetition and the forgetting curve",
    },
    SeedThought {
        body: "少即是多不是美学口号——它是认知负载理论的应用。每增加一个选项，用户做决定的时间指数增长（Hick's Law）。\n\nLess is more isn't an aesthetic slogan — it's applied cognitive load theory. Each additional option increases decision time exponentially (Hick's Law).",
        summary: "Hick's Law and cognitive load",
    },
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedResult {
    pub seeded: bool,
    pub thought_ids: Vec<String>,
}

#[tauri::command]
pub async fn seed_onboarding_content(
    state: tauri::State<'_, crate::WorkspaceState>,
) -> Result<SeedResult, String> {
    let root = crate::lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || seed_blocking(&root))
        .await
        .map_err(|e| e.to_string())?
}

fn seed_blocking(vault_root: &Path) -> Result<SeedResult, String> {
    let marker = vault_root.join(SEED_MARKER);
    if marker.exists() {
        return Ok(SeedResult {
            seeded: false,
            thought_ids: Vec::new(),
        });
    }

    let conn = vault_thoughts_db::open_thoughts_db(vault_root)?;
    let now = Utc::now().to_rfc3339();
    let mut ids = Vec::new();

    for seed in SEED_THOUGHTS {
        let thought_id = format!("thought-{}", uuid::Uuid::new_v4().simple());
        let note_stable_id = thought_id.clone();
        let note_rel_path = format!(".knowforge/standalone/{thought_id}");

        vault_thoughts_db::upsert_thought_body(
            &conn,
            &thought_id,
            &note_stable_id,
            &note_rel_path,
            seed.body,
            Some(seed.summary),
            "seedling",
            true,
            true,
            &now,
            &now,
            0,
            Some(&now),
        )?;
        ids.push(thought_id);
    }

    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create marker dir: {e}"))?;
    }
    fs::write(&marker, "seeded").map_err(|e| format!("write marker: {e}"))?;

    Ok(SeedResult {
        seeded: true,
        thought_ids: ids,
    })
}
