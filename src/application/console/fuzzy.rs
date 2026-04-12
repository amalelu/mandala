//! Case-insensitive subsequence fuzzy match, identical to the one
//! that used to live in `palette.rs`. Kept as its own module so the
//! scoring algorithm is easy to find and the unit tests have a
//! natural home.
//!
//! The algorithm rewards dense matches and penalises gaps + a late
//! first match:
//!
//!     +10 per matched char
//!     -1  per haystack char skipped before the first match
//!     -1  per haystack char skipped between consecutive matches
//!     +2  bonus per match immediately following a word boundary
//!
//! The perf floor is nanoseconds for the ~12-entry command registry,
//! so no real fuzzy-match library is warranted.

/// Returns `None` when any char of `query` can't be matched in
/// `haystack`, or `Some(score)` otherwise. Higher is better.
pub fn fuzzy_score(query: &str, haystack: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let h: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    let mut qi = 0usize;
    let mut score: i32 = 0;
    let mut first_match: Option<usize> = None;
    let mut prev_match: Option<usize> = None;
    for (hi, &hc) in h.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if hc == q[qi] {
            score += 10;
            if first_match.is_none() {
                first_match = Some(hi);
                score -= hi as i32;
            }
            if let Some(pm) = prev_match {
                let gap = (hi - pm - 1) as i32;
                score -= gap;
            }
            if hi == 0 || !h[hi - 1].is_alphanumeric() {
                score += 2;
            }
            prev_match = Some(hi);
            qi += 1;
        }
    }
    if qi == q.len() { Some(score) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_score_empty_query_returns_zero() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn test_fuzzy_score_subsequence_match() {
        assert!(fuzzy_score("rst", "reset").is_some());
        assert!(fuzzy_score("anc", "anchor set from auto").is_some());
    }

    #[test]
    fn test_fuzzy_score_missing_char_returns_none() {
        assert_eq!(fuzzy_score("xyz", "reset connection"), None);
    }

    #[test]
    fn test_fuzzy_score_case_insensitive() {
        assert!(fuzzy_score("TOP", "anchor set from top").is_some());
        assert!(fuzzy_score("top", "ANCHOR SET FROM TOP").is_some());
    }

    #[test]
    fn test_fuzzy_score_prefers_earlier_match() {
        let early = fuzzy_score("top", "top of list").unwrap();
        let late = fuzzy_score("top", "this is near the top").unwrap();
        assert!(early > late, "early={early} late={late}");
    }

    #[test]
    fn test_fuzzy_score_word_boundary_bonus() {
        let boundary = fuzzy_score("anchor", "set anchor side").unwrap();
        let inside = fuzzy_score("anchor", "setanchorside").unwrap();
        assert!(boundary > inside, "boundary={boundary} inside={inside}");
    }
}
