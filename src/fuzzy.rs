//! fzf-style fuzzy matching: smart-case subsequence matching with
//! word-boundary and consecutive-match bonuses. Space-separated terms must all match.

const MATCH: i64 = 16;
const BOUNDARY_BONUS: i64 = 12;
const CAMEL_BONUS: i64 = 8;
const CONSECUTIVE_BONUS: i64 = 10;
const FIRST_CHAR_MULTIPLIER: i64 = 2;
const GAP_START: i64 = 3;
const GAP_EXTENSION: i64 = 1;

/// A successful match: its score and the matched character indices of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    pub positions: Vec<usize>,
}

fn is_separator(c: char) -> bool {
    matches!(c, '/' | '-' | '_' | '.' | ' ' | ':')
}

fn eq(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive { a == b } else { a.to_lowercase().eq(b.to_lowercase()) }
}

fn position_bonus(text: &[char], j: usize) -> i64 {
    match j.checked_sub(1).map(|p| text[p]) {
        None => BOUNDARY_BONUS,
        Some(prev) if is_separator(prev) => BOUNDARY_BONUS,
        Some(prev) if prev.is_lowercase() && text[j].is_uppercase() => CAMEL_BONUS,
        _ => 0,
    }
}

/// Finds the best-scoring placement of a single term (no whitespace) in `text`.
fn match_term(text: &[char], term: &str) -> Option<Match> {
    let query: Vec<char> = term.chars().collect();
    let (n, m) = (text.len(), query.len());
    if m == 0 {
        return Some(Match { score: 0, positions: Vec::new() });
    }
    if m > n {
        return None;
    }
    // Smart case: uppercase in the query makes it case sensitive.
    let case_sensitive = query.iter().any(|c| c.is_uppercase());
    // best[i][j]: best score with query[i] matched at text[j]; parent for backtracking.
    let mut best = vec![vec![None::<i64>; n]; m];
    let mut parent = vec![vec![usize::MAX; n]; m];
    for i in 0..m {
        for j in i..n {
            if !eq(text[j], query[i], case_sensitive) {
                continue;
            }
            let mut bonus = position_bonus(text, j);
            if i == 0 {
                bonus *= FIRST_CHAR_MULTIPLIER;
                best[0][j] = Some(MATCH + bonus - gap_penalty(j));
                continue;
            }
            let mut candidate: Option<(i64, usize)> = None;
            #[allow(clippy::needless_range_loop)]
            #[allow(clippy::needless_range_loop)]
            for k in (i - 1)..j {
                let Some(previous) = best[i - 1][k] else { continue };
                let score = if k + 1 == j {
                    previous + MATCH + bonus.max(CONSECUTIVE_BONUS)
                } else {
                    previous + MATCH + bonus - gap_penalty(j - k - 1)
                };
                if candidate.is_none_or(|(best_score, _)| score > best_score) {
                    candidate = Some((score, k));
                }
            }
            if let Some((score, k)) = candidate {
                best[i][j] = Some(score);
                parent[i][j] = k;
            }
        }
    }
    let (mut j, score) = (0..n)
        .filter_map(|j| best[m - 1][j].map(|score| (j, score)))
        .max_by_key(|&(j, score)| (score, std::cmp::Reverse(j)))?;
    let mut positions = vec![0; m];
    for i in (0..m).rev() {
        positions[i] = j;
        j = parent[i][j];
    }
    Some(Match { score, positions })
}

fn gap_penalty(gap: usize) -> i64 {
    if gap == 0 { 0 } else { GAP_START + (gap as i64 - 1) * GAP_EXTENSION }
}

/// Matches `query` against `text`. Every space-separated term must match.
/// An empty query matches everything with a score of zero.
///
/// ```
/// use canopy::fuzzy::fuzzy_match;
/// assert!(fuzzy_match("cnpy", "github/canopy").is_some());
/// assert!(fuzzy_match("xyz", "github/canopy").is_none());
/// ```
pub fn fuzzy_match(query: &str, text: &str) -> Option<Match> {
    let chars: Vec<char> = text.chars().collect();
    let mut total = Match { score: 0, positions: Vec::new() };
    for term in query.split_whitespace() {
        let found = match_term(&chars, term)?;
        total.score += found.score;
        total.positions.extend(found.positions);
    }
    total.positions.sort_unstable();
    total.positions.dedup();
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_subsequence_in_order() {
        assert!(fuzzy_match("cnp", "canopy").is_some());
        assert!(fuzzy_match("pnc", "canopy").is_none());
        assert!(fuzzy_match("", "anything").is_some());
    }

    #[test]
    fn smart_case() {
        assert!(fuzzy_match("canopy", "CANOPY").is_some());
        assert!(fuzzy_match("Canopy", "canopy").is_none());
    }

    #[test]
    fn prefers_boundaries_and_consecutive_runs() {
        let tight = fuzzy_match("can", "work/canopy").unwrap().score;
        let loose = fuzzy_match("can", "work/c_a_n").unwrap().score;
        let scattered = fuzzy_match("can", "xcxaxnx").unwrap().score;
        assert!(tight > loose && loose > scattered);
    }

    #[test]
    fn terms_are_anded_and_positions_reported() {
        let found = fuzzy_match("gh can", "github/canopy").unwrap();
        assert!(found.positions.contains(&0) && found.positions.contains(&7));
        assert!(fuzzy_match("gh zzz", "github/canopy").is_none());
    }
}
