use super::{OVERVIEW_BATCH, SEARCH_BUDGET};

/// Finite near-to-far search. Cached spans are read, never refetched; successful
/// attempts include empty responses and consume the durable request budget.
pub(in crate::runtime) struct DiscoveryPlan {
    pub windows: Vec<(u64, u64)>,
    pub cached_windows: Vec<(u64, u64)>,
}

impl DiscoveryPlan {
    pub fn new(
        anchors: &[u64],
        group_low: u64,
        group_high: u64,
        stored: &[(u64, u64)],
        attempted: &[(u64, u64)],
    ) -> Self {
        if group_low > group_high || anchors.is_empty() {
            return Self {
                windows: Vec::new(),
                cached_windows: Vec::new(),
            };
        }
        let prior = merge(attempted);
        let used = prior.iter().fold(0u64, |sum, &(low, high)| {
            sum.saturating_add(high.saturating_sub(low).saturating_add(1))
        });
        let mut budget = SEARCH_BUDGET.saturating_sub(used);
        let mut covered = merge(&[stored, &prior].concat());
        let mut cached_windows = Vec::new();
        let mut cache_budget = SEARCH_BUDGET;
        let mut windows = Vec::new();
        for radius in [10_000u64, 100_000, 500_000] {
            let scope = merge(
                &anchors
                    .iter()
                    .map(|&article| {
                        (
                            article.saturating_sub(radius).max(group_low),
                            article.saturating_add(radius).min(group_high),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
            for &(low, high) in &scope {
                // Avoid rescanning a cached vicinity in every expansion stage.
                for (first, last) in missing(low, high, &merge(&cached_windows)) {
                    if cache_budget == 0 {
                        break;
                    }
                    let last = first.saturating_add(cache_budget - 1).min(last);
                    cached_windows.push((first, last));
                    cache_budget -= last - first + 1;
                }
                for (mut first, last) in missing(low, high, &covered) {
                    while first <= last && budget > 0 {
                        let end = first
                            .saturating_add(OVERVIEW_BATCH.min(budget) - 1)
                            .min(last);
                        windows.push((first, end));
                        covered.push((first, end));
                        budget -= end - first + 1;
                        if windows.len() >= 200 {
                            budget = 0;
                        }
                        if end == u64::MAX {
                            break;
                        }
                        first = end + 1;
                    }
                }
                covered = merge(&covered);
            }
            covered = merge(&covered);
        }
        Self {
            windows,
            cached_windows: merge(&cached_windows),
        }
    }
}

fn merge(ranges: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut ranges = ranges
        .iter()
        .copied()
        .filter(|(low, high)| low <= high)
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut result: Vec<(u64, u64)> = Vec::new();
    for (low, high) in ranges {
        if let Some(last) = result
            .last_mut()
            .filter(|last| low <= last.1.saturating_add(1))
        {
            last.1 = last.1.max(high);
        } else {
            result.push((low, high));
        }
    }
    result
}

fn missing(first: u64, last: u64, covered: &[(u64, u64)]) -> Vec<(u64, u64)> {
    if first > last {
        return Vec::new();
    }
    let mut cursor = first;
    let mut result = Vec::new();
    for &(low, high) in covered {
        if high < cursor {
            continue;
        }
        if low > last {
            break;
        }
        if low > cursor {
            result.push((cursor, low - 1));
        }
        if high >= last {
            return result;
        }
        cursor = cursor.max(high + 1);
    }
    result.push((cursor, last));
    result
}
