//! Shared wording for the two lower-bound emoji counts in `info`.
use figstudio_render::Stats;

pub(crate) fn summary(counts: &Stats) -> String {
    let qualifier = if counts.budget_exhausted > 0 || counts.depth_skipped > 0 {
        "at least "
    } else {
        ""
    };
    format!(
        "{qualifier}{} approximated, {qualifier}{} skipped",
        counts.emoji_approximated, counts.emoji_skipped
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_walk_qualifies_both_counts() {
        for (budget_exhausted, depth_skipped) in [(1, 0), (0, 1)] {
            let counts = Stats {
                emoji_approximated: 2,
                emoji_skipped: 3,
                budget_exhausted,
                depth_skipped,
                ..Stats::default()
            };
            assert_eq!(
                summary(&counts),
                "at least 2 approximated, at least 3 skipped"
            );
        }
        assert_eq!(summary(&Stats::default()), "0 approximated, 0 skipped");
    }
}
