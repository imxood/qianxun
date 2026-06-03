use std::collections::HashMap;

/// 向量条目。
#[derive(Debug, Clone)]
pub struct VectorEntry {
    pub embedding: Vec<f32>,
    pub dimensions: usize,
}

/// 运行时向量索引。
///
/// 持久化：通过 SQLite observation_vectors 表（BLOB 列），启动时全量加载。
/// 每次 add() 写入 SQLite，不做全量序列化。
#[allow(dead_code)] // dimensions 字段: 向量一致性校验待实现, 留作 Phase 3a 骨架
pub struct VectorIndex {
    vectors: HashMap<String, VectorEntry>,
    dimensions: usize,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self {
        Self {
            vectors: HashMap::new(),
            dimensions,
        }
    }

    /// 添加或更新向量。
    pub fn add(&mut self, id: String, embedding: Vec<f32>) {
        let dims = embedding.len();
        self.vectors.insert(
            id,
            VectorEntry {
                embedding,
                dimensions: dims,
            },
        );
    }

    /// 删除向量。
    pub fn remove(&mut self, id: &str) {
        self.vectors.remove(id);
    }

    /// 余弦相似度搜索。
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(String, f64)> {
        if self.vectors.is_empty() || query.is_empty() {
            return vec![];
        }

        let mut scores: Vec<(String, f64)> = self
            .vectors
            .iter()
            .map(|(id, entry)| {
                let score = cosine_similarity(query, &entry.embedding);
                (id.clone(), score)
            })
            .collect();

        // 按相似度降序排列
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(limit);
        scores
    }

    /// 索引大小。
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// 是否为空的索引。
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }
}

/// 余弦相似度计算。
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_search() {
        let mut idx = VectorIndex::new(3);
        idx.add("a".into(), vec![1.0, 0.0, 0.0]);
        idx.add("b".into(), vec![0.0, 1.0, 0.0]);
        idx.add("c".into(), vec![1.0, 1.0, 0.0]);

        // 搜索与 [1, 0, 0] 最相似的
        let results = idx.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert!((results[0].1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_empty_index() {
        let idx = VectorIndex::new(3);
        assert!(idx.is_empty());
        let results = idx.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_remove() {
        let mut idx = VectorIndex::new(2);
        idx.add("x".into(), vec![1.0, 0.0]);
        idx.remove("x");
        assert!(idx.is_empty());
    }
}
