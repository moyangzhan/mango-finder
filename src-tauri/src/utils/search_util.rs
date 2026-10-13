use crate::enums::QueryIntent;
use std::collections::HashSet;
use std::sync::LazyLock;

pub static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut s = HashSet::new();
    // English stop words (prepositions, conjunctions, articles)
    let en_words = [
        "a", "an", "the", "and", "or", "but", "if", "then", "else", "when", "at", "by", "from",
        "for", "in", "off", "on", "out", "over", "to", "with", "is", "was", "are", "were", "be",
        "been", "being", "has", "have", "had", "do", "does", "did", "of", "about",
    ];
    // Chinese stop words (particles, conjunctions, prepositions)
    let cn_words = [
        "的", "了", "在", "是", "我", "有", "和", "就", "不", "人", "都", "一", "一个", "上", "也",
        "很", "到", "说", "要", "去", "你", "会", "着", "没有", "看", "好", "自己", "这", "与",
        "于", "之",
    ];

    for word in en_words {
        s.insert(word);
    }
    for word in cn_words {
        s.insert(word);
    }
    s
});

pub fn detect_intent(query: &str) -> QueryIntent {
    let q = query.trim();

    if q.contains('\\') || q.contains('/') {
        return QueryIntent::PathOnly;
    }

    if q.contains('*') || q.contains('.') {
        return QueryIntent::PathOnly;
    }

    let word_count = q.split_whitespace().count();
    if word_count <= 2 {
        return QueryIntent::PathOnly;
    }

    let semantic_keywords = [
        "about", "related", "that", "which", "where", "notes", "document",
    ];

    if semantic_keywords.iter().any(|k| q.contains(k)) {
        return QueryIntent::Hybrid;
    }

    if q.len() > 20 {
        return QueryIntent::SemanticOnly;
    }

    QueryIntent::Hybrid
}
