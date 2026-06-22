use super::*;
use super::extraction::{
    build_memory_summary_for_extraction, build_reflection_prompt, build_session_extraction_prompt,
    trim_messages_for_extraction, truncate_message, MAX_EXTRACTION_MESSAGES,
};
use super::injection::estimate_tokens;
use super::merge::{MAX_CORRECTIONS, MAX_KNOWLEDGE_DOMAINS, MAX_SESSIONS};
use super::workspace::{evenly_spaced_indices, extract_yaml_tags, MAX_FREQUENT_PATHS};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use tempfile::TempDir;

use crate::llm::LlmChatMessage;

// -- Load / Save --

#[test]
fn default_memory_has_version_2() {
    let m = AgentMemory::default();
    assert_eq!(m.version, 2);
    assert!(m.knowledge_domains.is_empty());
    assert!(m.corrections.is_empty());
    assert!(m.sessions.is_empty());
}

#[test]
fn load_returns_default_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    let m = AgentMemory::load(tmp.path());
    assert_eq!(m.version, 2);
}

#[test]
fn save_and_load_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "use Chinese titles".to_string(),
        reason: "user preference".to_string(),
        date: "2026-06-15".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: Some("async".to_string()),
        motivation: None,
        confidence: 0.7,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 2,
        archived: false,
    });
    m.save(tmp.path()).unwrap();
    let loaded = AgentMemory::load(tmp.path());
    assert_eq!(loaded.corrections.len(), 1);
    assert_eq!(loaded.corrections[0].rule, "use Chinese titles");
    assert_eq!(loaded.knowledge_domains.len(), 1);
    assert_eq!(loaded.knowledge_domains[0].domain, "Rust");
    assert!((loaded.knowledge_domains[0].confidence - 0.7).abs() < f64::EPSILON);
}

#[test]
fn load_corrupt_json_returns_default() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(KNOWFORGE_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MEMORY_FILE), "not json{{{").unwrap();
    let m = AgentMemory::load(tmp.path());
    assert_eq!(m.version, 2);
}

#[test]
fn save_creates_knowforge_dir() {
    let tmp = TempDir::new().unwrap();
    let m = AgentMemory::default();
    m.save(tmp.path()).unwrap();
    assert!(tmp.path().join(KNOWFORGE_DIR).join(MEMORY_FILE).exists());
}

// -- Observe workspace --

#[test]
fn observe_empty_paths() {
    let tmp = TempDir::new().unwrap();
    let ws = observe_workspace(tmp.path(), &[]);
    assert!(ws.frequent_paths.is_empty());
    assert!(ws.language_distribution.is_empty());
    assert!(ws.updated_at.is_some());
}

#[test]
fn observe_language_distribution() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
    std::fs::write(
        tmp.path().join("notes/a.md"),
        "这是中文笔记的内容，包含很多中文字符",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("notes/b.md"),
        "另一篇中文笔记，同样以中文为主",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("notes/c.md"),
        "This is English content",
    )
    .unwrap();
    let paths = vec![
        "notes/a.md".to_string(),
        "notes/b.md".to_string(),
        "notes/c.md".to_string(),
    ];
    let ws = observe_workspace(tmp.path(), &paths);
    let zh = ws.language_distribution.get("zh").copied().unwrap_or(0.0);
    let en = ws.language_distribution.get("en").copied().unwrap_or(0.0);
    assert!(zh > 0.0, "should detect CJK characters");
    assert!(en > 0.0, "should detect ASCII characters");
    assert!(zh > en, "CJK should dominate with these inputs");
}

#[test]
fn observe_language_fallback_to_filename() {
    let tmp = TempDir::new().unwrap();
    let paths = vec![
        "notes/中文笔记.md".to_string(),
        "notes/另一个.md".to_string(),
        "notes/english.md".to_string(),
    ];
    let ws = observe_workspace(tmp.path(), &paths);
    let zh = ws.language_distribution.get("zh").copied().unwrap_or(0.0);
    assert!(
        zh > 0.0,
        "should fallback to filename-based CJK detection"
    );
}

#[test]
fn observe_frequent_paths() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("reading-notes")).unwrap();
    std::fs::create_dir_all(tmp.path().join("daily")).unwrap();
    std::fs::write(tmp.path().join("reading-notes/a.md"), "a").unwrap();
    std::fs::write(tmp.path().join("reading-notes/b.md"), "b").unwrap();
    std::fs::write(tmp.path().join("reading-notes/c.md"), "c").unwrap();
    std::fs::write(tmp.path().join("daily/d.md"), "d").unwrap();
    let paths = vec![
        "reading-notes/a.md".to_string(),
        "reading-notes/b.md".to_string(),
        "reading-notes/c.md".to_string(),
        "daily/d.md".to_string(),
    ];
    let ws = observe_workspace(tmp.path(), &paths);
    assert!(!ws.frequent_paths.is_empty());
    assert_eq!(ws.frequent_paths[0].path, "reading-notes/");
    assert!(ws.frequent_paths[0].description.contains("3"));
}

#[test]
fn observe_frequent_paths_truncated() {
    let tmp = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for i in 0..20 {
        let dir = format!("dir{i}");
        std::fs::create_dir_all(tmp.path().join(&dir)).unwrap();
        std::fs::write(tmp.path().join(format!("{dir}/note.md")), "x").unwrap();
        paths.push(format!("{dir}/note.md"));
    }
    let ws = observe_workspace(tmp.path(), &paths);
    assert!(ws.frequent_paths.len() <= MAX_FREQUENT_PATHS);
}

#[test]
fn observe_tag_vocabulary() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("notes")).unwrap();
    std::fs::write(
        tmp.path().join("notes/a.md"),
        "---\ntitle: A\ntags:\n  - rust\n  - async\n---\nContent here",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("notes/b.md"),
        "---\ntags: [rust, networking]\n---\nMore content",
    )
    .unwrap();
    let paths = vec!["notes/a.md".to_string(), "notes/b.md".to_string()];
    let ws = observe_workspace(tmp.path(), &paths);
    assert!(ws.tag_vocabulary.contains(&"rust".to_string()));
    assert!(ws.tag_vocabulary.contains(&"async".to_string()));
    assert!(ws.tag_vocabulary.contains(&"networking".to_string()));
}

#[test]
fn observe_topics_fallback_to_dirs() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("reading-notes")).unwrap();
    std::fs::write(tmp.path().join("reading-notes/a.md"), "content").unwrap();
    let paths = vec!["reading-notes/a.md".to_string()];
    let ws = observe_workspace(tmp.path(), &paths);
    assert!(ws.topics.contains(&"reading-notes".to_string()));
}

#[test]
fn extract_yaml_tags_list_form() {
    let mut out = HashSet::new();
    extract_yaml_tags(
        "title: X\ntags:\n  - alpha\n  - beta\ndate: 2026",
        &mut out,
    );
    assert!(out.contains("alpha"));
    assert!(out.contains("beta"));
    assert_eq!(out.len(), 2);
}

#[test]
fn extract_yaml_tags_inline_form() {
    let mut out = HashSet::new();
    extract_yaml_tags("tags: [foo, \"bar baz\", 'qux']", &mut out);
    assert!(out.contains("foo"));
    assert!(out.contains("bar baz"));
    assert!(out.contains("qux"));
}

#[test]
fn evenly_spaced_indices_small() {
    assert_eq!(evenly_spaced_indices(3, 10), vec![0, 1, 2]);
    assert_eq!(evenly_spaced_indices(0, 10), Vec::<usize>::new());
}

#[test]
fn evenly_spaced_indices_large() {
    let indices = evenly_spaced_indices(100, 5);
    assert_eq!(indices.len(), 5);
    assert_eq!(indices[0], 0);
    assert!(indices[4] < 100);
}

// -- Confidence decay --

#[test]
fn decay_within_30_days_no_change() {
    let mut m = AgentMemory::default();
    let today = Utc::now().format("%Y-%m-%d").to_string();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: today,
        evidence_count: 1,
        archived: false,
    });
    m.apply_confidence_decay();
    assert!((m.knowledge_domains[0].confidence - 0.7).abs() < f64::EPSILON);
    assert!(!m.knowledge_domains[0].archived);
}

#[test]
fn decay_after_60_days() {
    let mut m = AgentMemory::default();
    let old_date = (Utc::now().date_naive() - chrono::Duration::days(61))
        .format("%Y-%m-%d")
        .to_string();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: old_date,
        evidence_count: 1,
        archived: false,
    });
    m.apply_confidence_decay();
    assert!((m.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
}

#[test]
fn decay_archives_below_threshold() {
    let mut m = AgentMemory::default();
    let old_date = (Utc::now().date_naive() - chrono::Duration::days(150))
        .format("%Y-%m-%d")
        .to_string();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: old_date,
        evidence_count: 1,
        archived: false,
    });
    m.apply_confidence_decay();
    assert!(m.knowledge_domains[0].archived);
    assert!(m.knowledge_domains[0].confidence < 0.3);
}

#[test]
fn decay_high_evidence_slower() {
    let mut m = AgentMemory::default();
    let old_date = (Utc::now().date_naive() - chrono::Duration::days(61))
        .format("%Y-%m-%d")
        .to_string();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "practitioner".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: old_date.clone(),
        evidence_count: 10,
        archived: false,
    });
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Go".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: old_date,
        evidence_count: 1,
        archived: false,
    });
    m.apply_confidence_decay();
    let rust_conf = m.knowledge_domains[0].confidence;
    let go_conf = m.knowledge_domains[1].confidence;
    assert!(
        rust_conf > go_conf,
        "high evidence domain ({rust_conf}) should decay slower than low evidence ({go_conf})"
    );
}

#[test]
fn decay_skips_archived() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "old".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.1,
        last_evidence: "2020-01-01".to_string(),
        evidence_count: 1,
        archived: true,
    });
    m.apply_confidence_decay();
    assert!((m.knowledge_domains[0].confidence - 0.1).abs() < f64::EPSILON);
}

// -- Format for injection --

#[test]
fn injection_empty_memory_returns_none() {
    let m = AgentMemory::default();
    assert!(m.format_for_injection().is_none());
}

#[test]
fn injection_includes_high_confidence_directly() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "distributed systems".to_string(),
        depth: "practitioner".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.9,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 5,
        archived: false,
    });
    let text = m.format_for_injection().unwrap();
    assert!(text.contains("distributed systems (practitioner"));
    assert!(!text.contains("likely"));
    assert!(!text.contains("possibly"));
}

#[test]
fn injection_medium_confidence_uses_likely() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.6,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let text = m.format_for_injection().unwrap();
    assert!(text.contains("likely learning"));
}

#[test]
fn injection_low_confidence_uses_summary() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "quantum".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.35,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let text = m.format_for_injection().unwrap();
    assert!(text.contains("Also some interest in: quantum"));
}

#[test]
fn injection_below_threshold_not_injected() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "invisible".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.2,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 1,
        archived: false,
    });
    assert!(m.format_for_injection().is_none());
}

#[test]
fn injection_archived_not_injected() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "archived".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.8,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 3,
        archived: true,
    });
    assert!(m.format_for_injection().is_none());
}

#[test]
fn injection_corrections_format() {
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "use Chinese titles".to_string(),
        reason: "user prefers zh".to_string(),
        date: "2026-06-15".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    let text = m.format_for_injection().unwrap();
    assert!(text.contains("use Chinese titles"));
    assert!(text.contains("user prefers zh"));
}

#[test]
fn injection_sessions_shows_recent_3() {
    let mut m = AgentMemory::default();
    for i in 0..5 {
        m.sessions.push(MemorySession {
            date: format!("2026-06-{:02}T00:00:00Z", 10 + i),
            summary: format!("session {i}"),
            domains_touched: Vec::new(),
            follow_up: None,
        });
    }
    let text = m.format_for_injection().unwrap();
    assert!(!text.contains("session 0"));
    assert!(!text.contains("session 1"));
    assert!(text.contains("session 2"));
    assert!(text.contains("session 3"));
    assert!(text.contains("session 4"));
}

#[test]
fn injection_budget_trims_sessions() {
    let mut m = AgentMemory::default();
    for i in 0..15 {
        m.knowledge_domains.push(KnowledgeDomain {
            domain: format!("domain-{i} with a longer name for token usage"),
            depth: "practitioner".to_string(),
            current_focus: Some("some focus area here".to_string()),
            motivation: Some("motivation text for testing".to_string()),
            confidence: 0.8,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 3,
            archived: false,
        });
    }
    for i in 0..20 {
        m.corrections.push(MemoryCorrection {
            rule: format!("rule number {i} with some longer text for budget testing"),
            reason: format!("reason {i} is important for testing purposes"),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
            superseded_by: None,
            superseded_at: None,
        });
    }
    for i in 0..5 {
        m.sessions.push(MemorySession {
            date: format!("2026-06-{:02}T00:00:00Z", 10 + i),
            summary: format!("worked on feature {i} with detailed summary text"),
            domains_touched: Vec::new(),
            follow_up: Some(format!("continue feature {i}")),
        });
    }
    let text = m.format_for_injection().unwrap();
    for i in 0..20 {
        assert!(
            text.contains(&format!("rule number {i}")),
            "correction {i} must never be trimmed"
        );
    }
    assert!(
        !text.contains("session 2") || !text.contains("session 3"),
        "sessions should be trimmed when over budget"
    );
}

#[test]
fn injection_corrections_never_trimmed() {
    let mut m = AgentMemory::default();
    for i in 0..20 {
        m.corrections.push(MemoryCorrection {
            rule: format!("important rule {i}"),
            reason: format!("reason {i}"),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
            superseded_by: None,
            superseded_at: None,
        });
    }
    let text = m.format_for_injection().unwrap();
    for i in 0..20 {
        assert!(text.contains(&format!("important rule {i}")));
    }
}

#[test]
fn estimate_tokens_basic() {
    assert_eq!(estimate_tokens("hello"), 1);
    assert_eq!(estimate_tokens("你好世界"), 4);
    assert_eq!(estimate_tokens("hello你好"), 3);
    assert_eq!(estimate_tokens(""), 0);
}

// -- Merge: knowledge domains --

#[test]
fn merge_new_domain() {
    let mut m = AgentMemory::default();
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: Some("async".to_string()),
            motivation: None,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.knowledge_domains.len(), 1);
    assert_eq!(m.knowledge_domains[0].evidence_count, 1);
    assert!((m.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
}

#[test]
fn merge_existing_domain_accumulates() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: Some("async".to_string()),
            motivation: None,
            confidence: 0.7,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.knowledge_domains.len(), 1);
    assert_eq!(m.knowledge_domains[0].evidence_count, 2);
    // confidence: 0.5 + (1.0 - 0.5) * 0.15 = 0.575
    assert!((m.knowledge_domains[0].confidence - 0.575).abs() < f64::EPSILON);
    assert_eq!(m.knowledge_domains[0].depth, "learning");
    assert_eq!(
        m.knowledge_domains[0].current_focus.as_deref(),
        Some("async")
    );
}

#[test]
fn merge_confidence_progressive() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 1,
        archived: false,
    });
    for _ in 0..4 {
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "learning".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
    }
    assert!(m.knowledge_domains[0].confidence > 0.73);
    assert!(m.knowledge_domains[0].confidence < 0.75);
}

#[test]
fn merge_confidence_capped_at_095() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "expert".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.94,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 10,
        archived: false,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "expert".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.knowledge_domains[0].confidence <= 0.95);
}

#[test]
fn merge_depth_only_upgrades() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "practitioner".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.8,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 3,
        archived: false,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.knowledge_domains[0].depth, "practitioner");
}

#[test]
fn merge_depth_upgrade_requires_confidence() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "expert".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.knowledge_domains[0].depth, "learning");
}

#[test]
fn merge_domain_case_insensitive() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: "2026-06-14".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.knowledge_domains.len(), 1);
    assert_eq!(m.knowledge_domains[0].evidence_count, 2);
}

#[test]
fn merge_reactivates_archived() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.2,
        last_evidence: "2025-01-01".to_string(),
        evidence_count: 1,
        archived: true,
    });
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(!m.knowledge_domains[0].archived);
}

#[test]
fn merge_domains_capacity_overflow() {
    let mut m = AgentMemory::default();
    for i in 0..16 {
        m.knowledge_domains.push(KnowledgeDomain {
            domain: format!("domain-{i}"),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5 + (i as f64 * 0.02),
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
    }
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "new-domain".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.3,
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.knowledge_domains.len() <= MAX_KNOWLEDGE_DOMAINS);
}

// -- Merge: interaction style pending --

#[test]
fn merge_style_first_observation_goes_to_pending() {
    let mut m = AgentMemory::default();
    let mut updates = HashMap::new();
    updates.insert(
        "detail_preference".to_string(),
        Some("concise".to_string()),
    );
    let update = UserModelUpdate {
        interaction_style_updates: updates,
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.interaction_style.detail_preference.is_none());
    let entry = m
        .interaction_style
        .pending
        .get("detail_preference")
        .unwrap();
    assert_eq!(entry.value, "concise");
    assert!(!entry.observed_at.is_empty());
}

#[test]
fn merge_style_second_consistent_promotes() {
    let mut m = AgentMemory::default();
    m.interaction_style.pending.insert(
        "detail_preference".to_string(),
        PendingStyleEntry::new("concise".to_string()),
    );
    let mut updates = HashMap::new();
    updates.insert(
        "detail_preference".to_string(),
        Some("concise".to_string()),
    );
    let update = UserModelUpdate {
        interaction_style_updates: updates,
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(
        m.interaction_style.detail_preference.as_deref(),
        Some("concise")
    );
    assert!(!m
        .interaction_style
        .pending
        .contains_key("detail_preference"));
}

#[test]
fn merge_style_different_value_replaces_pending() {
    let mut m = AgentMemory::default();
    m.interaction_style.pending.insert(
        "detail_preference".to_string(),
        PendingStyleEntry::new("concise".to_string()),
    );
    let mut updates = HashMap::new();
    updates.insert(
        "detail_preference".to_string(),
        Some("detailed".to_string()),
    );
    let update = UserModelUpdate {
        interaction_style_updates: updates,
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.interaction_style.detail_preference.is_none());
    assert_eq!(
        m.interaction_style
            .pending
            .get("detail_preference")
            .map(|e| &e.value),
        Some(&"detailed".to_string())
    );
}

#[test]
fn merge_style_skips_when_already_set() {
    let mut m = AgentMemory::default();
    m.interaction_style.detail_preference = Some("concise".to_string());
    let mut updates = HashMap::new();
    updates.insert(
        "detail_preference".to_string(),
        Some("concise".to_string()),
    );
    let update = UserModelUpdate {
        interaction_style_updates: updates,
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.interaction_style.pending.is_empty());
}

#[test]
fn merge_style_substring_promotes() {
    let mut m = AgentMemory::default();
    m.interaction_style.pending.insert(
        "detail_preference".to_string(),
        PendingStyleEntry::new("concise".to_string()),
    );
    let mut updates = HashMap::new();
    updates.insert(
        "detail_preference".to_string(),
        Some("concise replies".to_string()),
    );
    let update = UserModelUpdate {
        interaction_style_updates: updates,
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(
        m.interaction_style.detail_preference.as_deref(),
        Some("concise replies")
    );
    assert!(!m
        .interaction_style
        .pending
        .contains_key("detail_preference"));
}

#[test]
fn pending_ttl_expires_old_entries() {
    let mut m = AgentMemory::default();
    let old_time = (Utc::now() - chrono::Duration::days(31)).to_rfc3339();
    m.interaction_style.pending.insert(
        "detail_preference".to_string(),
        PendingStyleEntry {
            value: "concise".to_string(),
            observed_at: old_time,
        },
    );
    m.interaction_style.pending.insert(
        "format".to_string(),
        PendingStyleEntry::new("markdown".to_string()),
    );
    m.expire_pending_styles();
    assert!(!m
        .interaction_style
        .pending
        .contains_key("detail_preference"));
    assert!(m.interaction_style.pending.contains_key("format"));
}

// -- Merge: corrections --

#[test]
fn merge_corrections_remove_then_add() {
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "old rule".to_string(),
        reason: "old".to_string(),
        date: "2026-06-14".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    let update = UserModelUpdate {
        remove_corrections: vec!["old rule".to_string()],
        new_corrections: vec![NewCorrection {
            rule: "new rule".to_string(),
            reason: "new reason".to_string(),
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.corrections.len(), 2);
    assert!(m.corrections[0].superseded_by.is_some());
    let active: Vec<_> = m.corrections.iter().filter(|c| c.is_active()).collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].rule, "new rule");
}

#[test]
fn merge_corrections_dedup() {
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "existing rule".to_string(),
        reason: "old reason".to_string(),
        date: "2026-06-14".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    let update = UserModelUpdate {
        new_corrections: vec![NewCorrection {
            rule: "existing rule".to_string(),
            reason: "updated reason".to_string(),
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.corrections.len(), 1);
    assert_eq!(m.corrections[0].reason, "updated reason");
}

#[test]
fn merge_corrections_capacity() {
    let mut m = AgentMemory::default();
    for i in 0..21 {
        m.corrections.push(MemoryCorrection {
            rule: format!("rule-{i}"),
            reason: "reason".to_string(),
            date: format!("2026-06-{:02}", (i % 28) + 1),
            source: "explicit".to_string(),
            superseded_by: None,
            superseded_at: None,
        });
    }
    let update = UserModelUpdate {
        new_corrections: vec![NewCorrection {
            rule: "new rule".to_string(),
            reason: "reason".to_string(),
        }],
        ..Default::default()
    };
    m.merge_user_model(update);
    let active = m.corrections.iter().filter(|c| c.is_active()).count();
    assert!(active <= MAX_CORRECTIONS);
}

#[test]
fn forget_soft_deletes_correction() {
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "old preference".to_string(),
        reason: "user said so".to_string(),
        date: "2026-06-14".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    let update = UserModelUpdate {
        remove_corrections: vec!["old preference".to_string()],
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.corrections.len(), 1);
    assert_eq!(
        m.corrections[0].superseded_by.as_deref(),
        Some("user_forget")
    );
    assert!(m.corrections[0].superseded_at.is_some());
}

#[test]
fn injection_skips_superseded_corrections() {
    let mut m = AgentMemory::default();
    m.corrections.push(MemoryCorrection {
        rule: "active rule".to_string(),
        reason: "keep".to_string(),
        date: "2026-06-15".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    m.corrections.push(MemoryCorrection {
        rule: "dead rule".to_string(),
        reason: "should not appear".to_string(),
        date: "2026-06-14".to_string(),
        source: "explicit".to_string(),
        superseded_by: Some("user_forget".to_string()),
        superseded_at: Some("2026-06-15".to_string()),
    });
    let text = m.format_for_injection().unwrap();
    assert!(text.contains("active rule"));
    assert!(!text.contains("dead rule"));
}

#[test]
fn capacity_counts_active_only() {
    let mut m = AgentMemory::default();
    for i in 0..19 {
        m.corrections.push(MemoryCorrection {
            rule: format!("active-{i}"),
            reason: "r".to_string(),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
            superseded_by: None,
            superseded_at: None,
        });
    }
    m.corrections.push(MemoryCorrection {
        rule: "superseded".to_string(),
        reason: "r".to_string(),
        date: "2026-06-14".to_string(),
        source: "explicit".to_string(),
        superseded_by: Some("capacity".to_string()),
        superseded_at: Some("2026-06-15".to_string()),
    });
    let update = UserModelUpdate {
        new_corrections: vec![
            NewCorrection {
                rule: "new-a".to_string(),
                reason: "r".to_string(),
            },
            NewCorrection {
                rule: "new-b".to_string(),
                reason: "r".to_string(),
            },
        ],
        ..Default::default()
    };
    m.merge_user_model(update);
    let active = m.corrections.iter().filter(|c| c.is_active()).count();
    assert!(active <= MAX_CORRECTIONS);
}

#[test]
fn superseded_cleanup_after_90_days() {
    let mut m = AgentMemory::default();
    let old_date = (Utc::now() - chrono::Duration::days(91))
        .format("%Y-%m-%d")
        .to_string();
    m.corrections.push(MemoryCorrection {
        rule: "expired".to_string(),
        reason: "r".to_string(),
        date: "2026-01-01".to_string(),
        source: "explicit".to_string(),
        superseded_by: Some("user_forget".to_string()),
        superseded_at: Some(old_date),
    });
    m.corrections.push(MemoryCorrection {
        rule: "recent superseded".to_string(),
        reason: "r".to_string(),
        date: "2026-06-10".to_string(),
        source: "explicit".to_string(),
        superseded_by: Some("user_forget".to_string()),
        superseded_at: Some(Utc::now().format("%Y-%m-%d").to_string()),
    });
    m.corrections.push(MemoryCorrection {
        rule: "active".to_string(),
        reason: "r".to_string(),
        date: "2026-06-15".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    m.expire_superseded_corrections();
    assert_eq!(m.corrections.len(), 2);
    assert!(m.corrections.iter().any(|c| c.rule == "recent superseded"));
    assert!(m.corrections.iter().any(|c| c.rule == "active"));
    assert!(!m.corrections.iter().any(|c| c.rule == "expired"));
}

// -- Merge: sessions --

#[test]
fn merge_session_appends() {
    let mut m = AgentMemory::default();
    let update = UserModelUpdate {
        session_summary: Some("did stuff".to_string()),
        session_domains_touched: vec!["Rust".to_string()],
        follow_up: Some("continue".to_string()),
        ..Default::default()
    };
    m.merge_user_model(update);
    assert_eq!(m.sessions.len(), 1);
    assert_eq!(m.sessions[0].summary, "did stuff");
    assert_eq!(m.sessions[0].follow_up.as_deref(), Some("continue"));
}

#[test]
fn merge_session_no_summary_skips() {
    let mut m = AgentMemory::default();
    let update = UserModelUpdate::default();
    m.merge_user_model(update);
    assert!(m.sessions.is_empty());
}

#[test]
fn merge_session_capacity() {
    let mut m = AgentMemory::default();
    for i in 0..11 {
        m.sessions.push(MemorySession {
            date: format!("2026-06-{:02}T00:00:00Z", (i % 28) + 1),
            summary: format!("session {i}"),
            domains_touched: Vec::new(),
            follow_up: None,
        });
    }
    let update = UserModelUpdate {
        session_summary: Some("new session".to_string()),
        ..Default::default()
    };
    m.merge_user_model(update);
    assert!(m.sessions.len() <= MAX_SESSIONS);
    assert_eq!(m.sessions.last().unwrap().summary, "new session");
}

// -- Reflection: should_reflect --

#[test]
fn should_reflect_empty_memory_returns_false() {
    let memory = AgentMemory::default();
    let messages: Vec<LlmChatMessage> = (0..5)
        .map(|i| LlmChatMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        })
        .collect();
    assert!(!should_reflect(&messages, &memory));
}

#[test]
fn should_reflect_short_conversation_returns_false() {
    let mut memory = AgentMemory::default();
    memory.corrections.push(MemoryCorrection {
        rule: "test".to_string(),
        reason: "test".to_string(),
        date: "2026-06-15".to_string(),
        source: "explicit".to_string(),
        superseded_by: None,
        superseded_at: None,
    });
    let messages = vec![
        LlmChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "assistant".to_string(),
            content: "hello".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".to_string(),
            content: "bye".to_string(),
            ..Default::default()
        },
    ];
    assert!(!should_reflect(&messages, &memory));
}

#[test]
fn should_reflect_sufficient_returns_true() {
    let mut memory = AgentMemory::default();
    memory.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let messages: Vec<LlmChatMessage> = (0..6)
        .map(|i| LlmChatMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        })
        .collect();
    assert!(should_reflect(&messages, &memory));
}

// -- Reflection: apply_single_proposal --

#[test]
fn proposal_add_knowledge_domain() {
    let mut m = AgentMemory::default();
    let proposal = MemoryProposal {
        id: "mp-1".to_string(),
        action: ProposalAction::Add,
        category: "knowledge_domain".to_string(),
        target: None,
        content: serde_json::json!({
            "domain": "Python",
            "depth": "practitioner",
            "confidence": 0.8
        }),
        reason: "user discussed Python".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.knowledge_domains.len(), 1);
    assert_eq!(m.knowledge_domains[0].domain, "Python");
}

#[test]
fn proposal_add_correction() {
    let mut m = AgentMemory::default();
    let proposal = MemoryProposal {
        id: "mp-2".to_string(),
        action: ProposalAction::Add,
        category: "correction".to_string(),
        target: None,
        content: serde_json::json!({
            "rule": "always use Chinese",
            "reason": "user preference"
        }),
        reason: "explicit instruction".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.corrections.len(), 1);
    assert_eq!(m.corrections[0].rule, "always use Chinese");
}

#[test]
fn proposal_update_domain() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: Some("async".to_string()),
        motivation: None,
        confidence: 0.7,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 2,
        archived: false,
    });
    let proposal = MemoryProposal {
        id: "mp-3".to_string(),
        action: ProposalAction::Update,
        category: "knowledge_domain".to_string(),
        target: Some("Rust".to_string()),
        content: serde_json::json!({
            "depth": "practitioner",
            "current_focus": "macros"
        }),
        reason: "user showed deeper expertise".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.knowledge_domains[0].depth, "practitioner");
    assert_eq!(
        m.knowledge_domains[0].current_focus.as_deref(),
        Some("macros")
    );
}

#[test]
fn proposal_update_respects_depth_upgrade_only() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "practitioner".to_string(),
        current_focus: Some("async".to_string()),
        motivation: None,
        confidence: 0.7,
        last_evidence: "2026-06-10".to_string(),
        evidence_count: 2,
        archived: false,
    });
    let proposal = MemoryProposal {
        id: "mp-dg".to_string(),
        action: ProposalAction::Update,
        category: "knowledge_domain".to_string(),
        target: Some("Rust".to_string()),
        content: serde_json::json!({
            "depth": "curious",
            "current_focus": "macros"
        }),
        reason: "downgrade test".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.knowledge_domains[0].depth, "practitioner");
    assert_eq!(
        m.knowledge_domains[0].current_focus.as_deref(),
        Some("macros")
    );
    assert_eq!(m.knowledge_domains[0].evidence_count, 3);
    assert_ne!(m.knowledge_domains[0].last_evidence, "2026-06-10");
}

#[test]
fn proposal_archive_domain() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "React".to_string(),
        depth: "curious".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.5,
        last_evidence: "2026-01-01".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let proposal = MemoryProposal {
        id: "mp-4".to_string(),
        action: ProposalAction::Archive,
        category: "knowledge_domain".to_string(),
        target: Some("React".to_string()),
        content: serde_json::json!({}),
        reason: "user moved to Vue".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert!(m.knowledge_domains[0].archived);
}

#[test]
fn proposal_merge_archives_old_and_adds_new() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "frontend".to_string(),
        depth: "learning".to_string(),
        current_focus: Some("React".to_string()),
        motivation: None,
        confidence: 0.6,
        last_evidence: "2026-03-01".to_string(),
        evidence_count: 2,
        archived: false,
    });
    let proposal = MemoryProposal {
        id: "mp-5".to_string(),
        action: ProposalAction::Merge,
        category: "knowledge_domain".to_string(),
        target: Some("frontend".to_string()),
        content: serde_json::json!({
            "domain": "frontend",
            "depth": "practitioner",
            "current_focus": "Vue",
            "confidence": 0.7
        }),
        reason: "merge React and Vue into single frontend entry".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.knowledge_domains.len(), 1);
    let entry = &m.knowledge_domains[0];
    assert!(!entry.archived);
    assert_eq!(entry.current_focus.as_deref(), Some("Vue"));
}

#[test]
fn proposal_skip_no_change() {
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: "2026-06-15".to_string(),
        evidence_count: 1,
        archived: false,
    });
    let before = m.knowledge_domains.clone();
    let proposal = MemoryProposal {
        id: "mp-6".to_string(),
        action: ProposalAction::Skip,
        category: "knowledge_domain".to_string(),
        target: Some("Rust".to_string()),
        content: serde_json::json!({}),
        reason: "already captured".to_string(),
    };
    apply_single_proposal(&mut m, &proposal).unwrap();
    assert_eq!(m.knowledge_domains.len(), before.len());
    assert_eq!(m.knowledge_domains[0].depth, before[0].depth);
}

#[test]
fn reflection_prompt_contains_both_inputs() {
    let m = AgentMemory::default();
    let update = UserModelUpdate {
        knowledge_domains: vec![DomainUpdate {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
        }],
        ..Default::default()
    };
    let prompt = build_reflection_prompt(&m, &update);
    assert!(prompt.contains("Existing memory"));
    assert!(prompt.contains("New extraction"));
    assert!(prompt.contains("Rust"));
}

// -- trim_messages_for_extraction --

#[test]
fn trim_preserves_system_messages_and_first_user() {
    let mut msgs = Vec::new();
    msgs.push(LlmChatMessage {
        role: "system".to_string(),
        content: "sys1".to_string(),
        ..Default::default()
    });
    msgs.push(LlmChatMessage {
        role: "system".to_string(),
        content: "sys2".to_string(),
        ..Default::default()
    });
    msgs.push(LlmChatMessage {
        role: "user".to_string(),
        content: "first user".to_string(),
        ..Default::default()
    });
    for i in 0..40 {
        msgs.push(LlmChatMessage {
            role: if i % 2 == 0 { "assistant" } else { "user" }.to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        });
    }
    let trimmed = trim_messages_for_extraction(&msgs);
    assert!(trimmed.len() <= MAX_EXTRACTION_MESSAGES);
    assert_eq!(trimmed[0].content, "sys1");
    assert_eq!(trimmed[1].content, "sys2");
    assert_eq!(trimmed[2].content, "first user");
    assert_eq!(
        trimmed.last().unwrap().content,
        msgs.last().unwrap().content
    );
}

#[test]
fn trim_short_conversation_unchanged() {
    let msgs: Vec<LlmChatMessage> = (0..5)
        .map(|i| LlmChatMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        })
        .collect();
    let trimmed = trim_messages_for_extraction(&msgs);
    assert_eq!(trimmed.len(), 5);
}

// -- Snapshot & pending proposals --

#[test]
fn snapshot_create_and_delete() {
    let tmp = TempDir::new().unwrap();
    let m = AgentMemory::default();
    create_snapshot(tmp.path(), &m).unwrap();
    assert!(tmp.path().join(KNOWFORGE_DIR).join(SNAPSHOT_FILE).exists());

    delete_snapshot(tmp.path());
    assert!(!tmp.path().join(KNOWFORGE_DIR).join(SNAPSHOT_FILE).exists());
}

#[test]
fn snapshot_delete_nonexistent_no_panic() {
    let tmp = TempDir::new().unwrap();
    delete_snapshot(tmp.path());
}

#[test]
fn pending_proposals_roundtrip() {
    let tmp = TempDir::new().unwrap();

    let batch = MemoryProposalBatch {
        session_id: "test-session".to_string(),
        proposals: vec![MemoryProposal {
            id: "mp-1".to_string(),
            action: ProposalAction::Add,
            category: "knowledge_domain".to_string(),
            target: None,
            content: serde_json::json!({"domain": "Rust", "depth": "learning", "confidence": 0.5}),
            reason: "test".to_string(),
        }],
        created_at: Utc::now().to_rfc3339(),
    };

    save_pending_proposals(tmp.path(), &batch).unwrap();
    assert!(tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());

    let loaded = load_pending_proposals(tmp.path()).unwrap();
    assert_eq!(loaded.session_id, "test-session");
    assert_eq!(loaded.proposals.len(), 1);
    assert_eq!(loaded.proposals[0].id, "mp-1");
}

#[test]
fn pending_proposals_expired_returns_none() {
    let tmp = TempDir::new().unwrap();

    let old_date = (Utc::now() - chrono::Duration::days(8)).to_rfc3339();
    let batch = MemoryProposalBatch {
        session_id: "old-session".to_string(),
        proposals: vec![],
        created_at: old_date,
    };

    save_pending_proposals(tmp.path(), &batch).unwrap();
    assert!(tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());

    let loaded = load_pending_proposals(tmp.path());
    assert!(loaded.is_none());
    assert!(!tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());
}

#[test]
fn pending_proposals_save_and_delete() {
    let tmp = TempDir::new().unwrap();
    assert!(!tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());

    let batch = MemoryProposalBatch {
        session_id: "s".to_string(),
        proposals: vec![],
        created_at: Utc::now().to_rfc3339(),
    };
    save_pending_proposals(tmp.path(), &batch).unwrap();
    assert!(tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());

    delete_pending_proposals(tmp.path());
    assert!(!tmp.path().join(KNOWFORGE_DIR).join(PENDING_FILE).exists());
}

// -- MemoryManager --

#[test]
fn manager_new_loads_and_decays() {
    let tmp = TempDir::new().unwrap();
    let old_date = (Utc::now().date_naive() - chrono::Duration::days(61))
        .format("%Y-%m-%d")
        .to_string();
    let mut m = AgentMemory::default();
    m.knowledge_domains.push(KnowledgeDomain {
        domain: "Rust".to_string(),
        depth: "learning".to_string(),
        current_focus: None,
        motivation: None,
        confidence: 0.7,
        last_evidence: old_date,
        evidence_count: 1,
        archived: false,
    });
    m.save(tmp.path()).unwrap();

    let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
    assert!((mgr.memory.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
    assert!(!mgr.is_dirty());
}

#[test]
fn manager_new_missing_file() {
    let tmp = TempDir::new().unwrap();
    let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
    assert_eq!(mgr.memory.version, 2);
    assert!(mgr.memory.knowledge_domains.is_empty());
    assert!(!mgr.is_dirty());
}

#[test]
fn manager_dirty_flag() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
    assert!(!mgr.is_dirty());
    mgr.dirty = true;
    assert!(mgr.is_dirty());
    mgr.reset_dirty();
    assert!(!mgr.is_dirty());
}

#[tokio::test]
async fn extract_session_update_short_conversation_skips() {
    let tmp = TempDir::new().unwrap();
    let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
    let messages = vec![LlmChatMessage {
        role: "user".to_string(),
        content: "hello".to_string(),
        ..Default::default()
    }];
    let result = mgr.extract_session_update(&messages).await;
    assert!(matches!(result, Ok(None)));
}

#[tokio::test]
async fn extract_session_update_no_cloud_skips() {
    let tmp = TempDir::new().unwrap();
    let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
    let messages = vec![
        LlmChatMessage {
            role: "user".to_string(),
            content: "first".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "assistant".to_string(),
            content: "reply".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".to_string(),
            content: "second".to_string(),
            ..Default::default()
        },
    ];
    let result = mgr.extract_session_update(&messages).await;
    assert!(matches!(result, Ok(None)));
}

// -- trim_messages_for_extraction (additional) --

#[test]
fn trim_within_limit() {
    let msgs: Vec<LlmChatMessage> = (0..10)
        .map(|i| LlmChatMessage {
            role: "user".to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        })
        .collect();
    let trimmed = trim_messages_for_extraction(&msgs);
    assert_eq!(trimmed.len(), 10);
}

#[test]
fn trim_over_limit() {
    let msgs: Vec<LlmChatMessage> = (0..40)
        .map(|i| LlmChatMessage {
            role: "user".to_string(),
            content: format!("msg {i}"),
            ..Default::default()
        })
        .collect();
    let trimmed = trim_messages_for_extraction(&msgs);
    assert_eq!(trimmed.len(), MAX_EXTRACTION_MESSAGES);
    assert_eq!(trimmed[0].content, "msg 0");
    assert_eq!(trimmed[1].content, "msg 11");
    assert_eq!(trimmed.last().unwrap().content, "msg 39");
}

// -- Prompt builders --

#[test]
fn session_prompt_contains_memory_and_conversation() {
    let m = AgentMemory::default();
    let msgs = vec![
        LlmChatMessage {
            role: "user".to_string(),
            content: "tell me about Rust".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "assistant".to_string(),
            content: "Rust is a systems language".to_string(),
            ..Default::default()
        },
    ];
    let prompt = build_session_extraction_prompt(&m, &msgs);
    assert!(prompt.contains("No existing memory."));
    assert!(prompt.contains("[user]: tell me about Rust"));
    assert!(prompt.contains("[assistant]: Rust is a systems language"));
    assert!(prompt.contains("knowledge_domains"));
}

#[test]
fn memory_summary_includes_domains_and_corrections() {
    let m = AgentMemory {
        knowledge_domains: vec![
            KnowledgeDomain {
                domain: "Rust".to_string(),
                depth: "practitioner".to_string(),
                current_focus: Some("async".to_string()),
                motivation: None,
                confidence: 0.8,
                last_evidence: "2026-06-01".to_string(),
                evidence_count: 3,
                archived: false,
            },
            KnowledgeDomain {
                domain: "Python".to_string(),
                depth: "expert".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5,
                last_evidence: "2026-05-01".to_string(),
                evidence_count: 1,
                archived: true,
            },
        ],
        corrections: vec![MemoryCorrection {
            rule: "Use concise style".to_string(),
            reason: "user preference".to_string(),
            date: "2026-06-01".to_string(),
            source: "explicit".to_string(),
            superseded_by: None,
            superseded_at: None,
        }],
        interaction_style: InteractionStyle {
            detail_preference: Some("concise".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    let summary = build_memory_summary_for_extraction(&m);
    assert!(summary.contains("Rust (practitioner)"));
    assert!(summary.contains("focus: async"));
    assert!(
        !summary.contains("Python"),
        "archived domain must be excluded"
    );
    assert!(summary.contains("Use concise style"));
    assert!(summary.contains("detail_preference=concise"));
    assert!(!summary.contains("confidence"));
    assert!(!summary.contains("evidence_count"));
}

#[test]
fn session_prompt_filters_tool_messages() {
    let m = AgentMemory::default();
    let msgs = vec![
        LlmChatMessage {
            role: "user".to_string(),
            content: "search something".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "tool".to_string(),
            content: "tool result".to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "assistant".to_string(),
            content: "here's what I found".to_string(),
            ..Default::default()
        },
    ];
    let prompt = build_session_extraction_prompt(&m, &msgs);
    assert!(!prompt.contains("[tool]"));
    assert!(prompt.contains("[user]"));
    assert!(prompt.contains("[assistant]"));
}

// -- truncate_message --

#[test]
fn truncate_short_message() {
    assert_eq!(truncate_message("hello", 10), "hello");
}

#[test]
fn truncate_long_message() {
    let long = "a".repeat(600);
    let truncated = truncate_message(&long, 500);
    assert_eq!(truncated.len(), 500);
}

#[test]
fn truncate_multibyte() {
    let msg = "你好世界这是一个测试";
    let truncated = truncate_message(msg, 3);
    assert_eq!(truncated, "你好世");
}

#[test]
fn user_model_update_accepts_corrections_alias() {
    let json = r#"{
        "corrections": [{"rule": "use concise style", "reason": "user preference"}],
        "forget_corrections": ["old rule"]
    }"#;
    let update: UserModelUpdate = serde_json::from_str(json).unwrap();
    assert_eq!(update.new_corrections.len(), 1);
    assert_eq!(update.new_corrections[0].rule, "use concise style");
    assert_eq!(update.remove_corrections, vec!["old rule"]);
}
