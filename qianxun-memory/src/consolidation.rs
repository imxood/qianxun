use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Consolidation 管线 —— 将 Observation 聚类生成持久 Memory。
///
/// 每次 Session End 时触发：
/// 1. 扫描当前 session 的 Observation
/// 2. 按 concepts 集合 Jaccard > 0.5 聚类
/// 3. 平均 importance > 6 → 生成 Memory
/// 4. Jaccard > 0.7 → 版本升级
///
/// Phase C 收尾: 加顶层 `run_consolidation(db: &Arc<Mutex<Connection>>, ...)`,
/// 内部获取锁, 让 MemoryCore.session_end() 可以无脑调.
/// 内部 helper `run_consolidation_locked(conn: &Connection, ...)` 接受已加锁连接
/// 供测试 / 嵌套调用用.
pub fn run_consolidation(db: &Arc<Mutex<Connection>>, session_id: &str) {
    let conn = match db.lock() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[memory] consolidation db lock poisoned: {e}");
            return;
        }
    };
    run_consolidation_locked(&conn, session_id);
}

/// 已加锁连接的 consolidation 入口.
pub fn run_consolidation_locked(conn: &Connection, session_id: &str) {
    // 获取 session 的所有 observation
    let observations = match load_observations(conn, session_id) {
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
            if let Err(e) = save_memory(conn, &cluster, session_id) {
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
///
/// Phase C 收尾: 修 pre-existing 迭代中 mutate vec 的 bug. 原实现用
/// `for i in (0..len).rev()` + `clusters.remove(j)`, 当 i > j 移除 j 后,
/// i 索引漂移但 for loop range 不变, 下次访问 `clusters[i]` 时越界 panic.
/// 改成 while 循环 + j 跟随 shift 重置 (删除 j 后, 新元素到 j 位置, j 不递增).
fn merge_similar_clusters(clusters: &mut Vec<Cluster>) {
    let mut i = 0;
    while i < clusters.len() {
        let mut j = i + 1;
        while j < clusters.len() {
            let sim = jaccard_similarity(&clusters[i].concepts, &clusters[j].concepts);
            if sim > 0.3 {
                // 合并 j 到 i. j 位置被新元素 (原 j+1) 占据, j 不递增.
                let other = clusters.remove(j);
                clusters[i].concepts.extend(other.concepts);
                clusters[i].observations.extend(other.observations);
            } else {
                j += 1;
            }
        }
        i += 1;
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
