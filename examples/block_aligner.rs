use itertools::Itertools;
use pairwise_aligner::{prelude::*, *};

// Compare with block aligner:
// They do 10k pairs of length 10k and distance 10% in 2s!
fn main() {
    let ns = [10_000];
    let es = [0.10];

    for (&n, e) in ns.iter().cartesian_product(es) {
        for l in [8, 9, 10, 11, 12] {
            {
                let h = SeedHeuristic {
                    match_config: MatchConfig {
                        length: Fixed(l),
                        max_match_cost: 1,
                        ..MatchConfig::default()
                    },
                    distance_function: CountHeuristic,
                    pruning: true,
                    build_fast: false,
                    query_fast: QueryMode::Off,
                };
                let (a, b, alphabet, stats) = setup(n, e);
                align(&a, &b, &alphabet, stats, h)
            }
            .print();
        }
    }
}
