/// Truncate and normalize embedding vector to target dimension.
/// Uses Matryoshka truncation: simply take the first N dimensions, then apply L2 normalization to ensure vector magnitude is 1.
///
/// 将嵌入向量截断并归一化到目标维度。
/// 使用 Matryoshka 截断：直接取前 N 维，然后进行 L2 归一化确保向量模长为 1。
pub fn finalize_metadata_embedding(raw_weights: Vec<f32>, target_dim: usize) -> Vec<f32> {
    let mut truncated = raw_weights;
    truncated.truncate(target_dim);

    let squared_sum: f32 = truncated.iter().map(|x| x * x).sum();
    let norm = squared_sum.sqrt();

    if norm > f32::EPSILON {
        truncated.into_iter().map(|x| x / norm).collect()
    } else {
        truncated
    }
}

/// Calculate hybrid score for content search (dense + sparse).
///
/// # Arguments
/// - `distance`: Cosine distance (0.0 = identical, 1.0 = opposite)
/// - `sparse_score`: Sparse vector match score (0.0 - 1.0)
/// - `is_short_query`: Whether the query is short (≤2 tokens)
///
/// # Returns
/// Score in range 0-100
///
/// # Strategy
/// - Short query: Strict keyword matching requirement
/// - Long query: Weaker constraints, trust dense similarity more
///
/// 计算内容搜索的混合分数（稠密向量 + 稀疏向量）。
///
/// # 参数
/// - `distance`: 余弦距离（0.0 = 相同，1.0 = 相反）
/// - `sparse_score`: 稀疏向量匹配分数（0.0 - 1.0）
/// - `is_short_query`: 是否为短查询（≤2 个词）
///
/// # 返回值
/// 0-100 范围的分数
///
/// # 策略
/// - 短查询：严格要求关键词匹配
/// - 长查询：弱约束，更信任稠密相似度
pub fn calculate_content_score(distance: f32, sparse_score: f32, is_short_query: bool) -> usize {
    let dense_sim = (1.0 - distance.min(1.0)).max(0.0).min(1.0);

    // Filter out unrelated results
    if dense_sim < 0.1 {
        return (sparse_score * 50.0) as usize;
    }

    let hybrid_score = if is_short_query {
        // Short query mode: strict keyword validation
        if sparse_score > 0.15 {
            0.5 + (dense_sim * 0.5)
        } else if dense_sim > 0.65 {
            // High semantic + weak keyword: synonym compensation
            0.3 + (dense_sim * 0.35)
        } else {
            dense_sim * 0.2
        }
    } else {
        // Long query mode: weaker constraints
        if sparse_score > 0.15 {
            (dense_sim * 1.0 + 0.05).min(1.0)
        } else if sparse_score > 0.02 {
            dense_sim
        } else {
            dense_sim * 0.85
        }
    };

    (hybrid_score * 100.0).round().clamp(0.0, 100.0) as usize
}

/// Calculate hybrid score for metadata (title/filename) search.
///
/// # Arguments
/// - `distance`: Cosine distance (0.0 = identical, 1.0 = opposite)
/// - `sparse_score`: Sparse vector match score (0.0 - 1.0)
///
/// # Returns
/// Score in range 0-100
///
/// # Strategy
/// Metadata search prioritizes exact keyword matches in titles.
/// Formula: 20% semantic + 40% base boost + 40% lexical strength
///
/// 计算元数据（标题/文件名）搜索的混合分数。
///
/// # 参数
/// - `distance`: 余弦距离（0.0 = 相同，1.0 = 相反）
/// - `sparse_score`: 稀疏向量匹配分数（0.0 - 1.0）
///
/// # 返回值
/// 0-100 范围的分数
///
/// # 策略
/// 元数据搜索优先考虑标题中的精确关键词匹配。
/// 公式：20% 语义 + 40% 基础奖励 + 40% 词法强度
pub fn calculate_metadata_score(distance: f32, sparse_score: f32) -> usize {
    let dense_sim = (1.0 - distance.min(1.0)).max(0.0).min(1.0);

    // Filter out unrelated results
    if dense_sim < 0.1 {
        return (sparse_score * 50.0) as usize;
    }

    let hybrid_score = if sparse_score < 0.01 {
        // No keyword match: heavy penalty
        dense_sim * 0.1
    } else {
        let base_boost = 0.4;
        // tanh amplifies low sparse scores
        let sparse_contribution = (sparse_score * 20.0).tanh() * 0.4;
        dense_sim * 0.2 + base_boost + sparse_contribution
    };

    (hybrid_score * 100.0).round().clamp(0.0, 100.0) as usize
}
