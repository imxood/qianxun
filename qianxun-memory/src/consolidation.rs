use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::Arc;

/// Consolidation 管线 —— 将 Observation 聚类生成持久 Memory。
///
/// 每次 Session End 时触发：
/// 1. 扫描当前 session 的 Observation
/// 2. 按 concepts 集合 Jaccard > 0.5 聚类
/// 3. 平均 importance > 6 → 生成 Memory
/// 4. Jaccard > 0.7 → 版本升级
pub fn run_consolidation(db: &Arc<Connection>, session_id: &str) {
    // 获取 session 的所有 observation
    let observations = match load_observations(db, session_id) {
        Some(o) => o,
        None => return,
    };

    if observations.is_empty() {
        return;
    }

    // 聚类
    let clusters = cluster_observations(&observations);

    // 评估每个簇
    for cluster in clusters {
        if cluster.avg_importance() >= 6.0 || cluster.size() >= 3 {
            if let Err(e) = save_memory(db, &cluster, session_id) {
                tracing::warn!("[memory] consolidation save failed: {e}");
            }
        }
    }
}

/// 加载 session 的 observations。
fn load_observations(
    db: &Connection,
    session_id: &str,
) -> Option<Vec<ClusterObservation>> {
    let mut stmt = db
        .prepare(
            "SELECT id, json_extract(data, '$.title'),
                    json_extract(data, '$.narrative'),
                    json_extract(data, '$.concepts'),
                    json_extract(data, '$.files'),
                    json_extract(data, '$.importance')
             FROM observations
             WHERE session_id = ?1
             ORDER BY timestamp",
        )
        .ok()?;

    let obs: Vec<ClusterObservation> = stmt
        .query_map(params![session_id], |row| {
            let id: String = row.get(0)?;

            // json_extract returns JSON strings with quotes, remove them
            let title: String = row.get(1).unwrap_or_default();
            let narrative: String = row.get(2).unwrap_or_default();
            let concepts_json: String = row.get(3).unwrap_or_else(|_| "[]".into());
            let _files_json: String = row.get(4).unwrap_or_else(|_| "[]".into());
            let importance: u8 = row.get(5).unwrap_or(0);

            let concepts: Vec<String> = serde_json::from_str(&concepts_json).unwrap_or_default();

            Ok(ClusterObservation {
                id,
                title: title.trim_matches('"').to_string(),
                narrative: narrative.trim_matches('"').to_string(),
                concepts: concepts.into_iter().collect(),
                importance,
            })
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    Some(obs)
}

/// 一次 Observation 的聚类信息。
#[derive(Debug)]
struct ClusterObservation {
    id: String,
    title: String,
    narrative: String,
    concepts: HashSet<String>,
    importance: u8,
}

/// Observation 簇。
#[derive(Debug)]
struct Cluster {
    observations: Vec<ClusterObservation>,
    concepts: HashSet<String>,
}

impl Cluster {
    fn avg_importance(&self) -> f64 {
        if self.observations.is_empty() {
            return 0.0;
        }
        self.observations
            .iter()
            .map(|o| o.importance as f64)
            .sum::<f64>()
            / self.observations.len() as f64
    }

    fn size(&self) -> usize {
        self.observations.len()
    }
}

/// Jaccard 相似度计算。
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// 按 concepts 集合 Jaccard > 0.5 聚类。
fn cluster_observations(observations: &[ClusterObservation]) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();

    for obs in observations {
        let mut best_idx = None;

        for (idx, cluster) in clusters.iter().enumerate() {
            let sim = jaccard_similarity(&obs.concepts, &cluster.concepts);
            if sim > 0.5 {
                best_idx = Some(idx);
                break;
            }
        }

        match best_idx {
            Some(idx) => {
                let cluster = &mut clusters[idx];
                // 合并概念
                for c in &obs.concepts {
                    cluster.concepts.insert(c.clone());
                }
                cluster.observations.push(ClusterObservation {
                    id: obs.id.clone(),
                    title: obs.title.clone(),
                    narrative: obs.narrative.clone(),
                    concepts: obs.concepts.clone(),
                    importance: obs.importance,
                });
            }
            None => {
                clusters.push(Cluster {
                    concepts: obs.concepts.clone(),
                    observations: vec![ClusterObservation {
                        id: obs.id.clone(),
                        title: obs.title.clone(),
                        narrative: obs.narrative.clone(),
                        concepts: obs.concepts.clone(),
                        importance: obs.importance,
                    }],
                });
            }
        }
    }

    // 合并相似簇（二次扫描）
    merge_similar_clusters(&mut clusters);
    clusters
}

/// 二次扫描：合并相似簇。
fn merge_similar_clusters(clusters: &mut Vec<Cluster>) {
    let mut merged = true;
    while merged {
        merged = false;
        for i in (0..clusters.len()).rev() {
            for j in (0..clusters.len()).rev() {
                if i == j {
                    continue;
                }
                let sim = jaccard_similarity(&clusters[i].concepts, &clusters[j].concepts);
                if sim > 0.3 {
                    // 合并 j 到 i
                    let other = clusters.remove(j);
                    let target = &mut clusters[i];
                    for c in &other.concepts {
                        target.concepts.insert(c.clone());
                    }
                    target.observations.extend(other.observations);
                    merged = true;
                    break;
                }
            }
            if merged {
                break;
            }
        }
    }
}

/// 将簇保存为 Memory。
fn save_memory(db: &Connection, cluster: &Cluster, _session_id: &str) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();

    // 构建 Memory 数据
    let main_obs = &cluster.observations[0];
    let data = serde_json::json!({
        "title": main_obs.title.clone(),
        "content": main_obs.narrative.clone(),
        "concepts": cluster.concepts.iter().cloned().collect::<Vec<_>>(),
        "files": [],
        "strength": 5,
        "version": 1,
        "is_latest": true,
        "project": null,
        "access_count": 0,
    });

    let mem_id = format!("mem_{}", uuid::Uuid::new_v4());

    db.execute(
        "INSERT INTO memories (id, created_at, updated_at, mem_type, data)
         VALUES (?1, ?2, ?2, 'pattern', ?3)",
        params![mem_id, now, data.to_string()],
    )?;

    tracing::info!(
        "[memory] consolidated {} observations into memory '{}'",
        cluster.observations.len(),
        main_obs.title
    );

    Ok(())
}
