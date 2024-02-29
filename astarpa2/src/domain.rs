// TODO
// - Store block of blocks in a single allocation. Update `NwBlock` to contain multiple columns as once and be reusable.
// - timings
// - meet in the middle with A* and pruning on both sides
// - try jemalloc/mimalloc
// - Matches:
//   - Recursively merge matches to find r=2^k matches.
//     - possibly reduce until no more spurious matches
//     - tricky: requires many 'shadow' matches. Handle in cleaner way?
//  - Figure out why pruning up to Layer::MAX gives errors, but pruning up to highest_modified_contour does not.
// - QgramIndex for short k.
// - Analyze local doubling better
// - Speed up j_range more???
// BUG: Figure out why the delta=64 is broken in fixed_j_range.
mod local_doubling;

use super::*;
use crate::{block::Block, blocks::Blocks};
use pa_affine_types::AffineCost;
use pa_heuristic::*;
use pa_types::*;
use pa_vis_types::*;
use std::cmp::{max, min};
use Domain::*;

pub struct AstarPa2Instance<'a, V: VisualizerT, H: Heuristic> {
    // NOTE: `a` and `b` are padded sequences and hence owned.
    pub a: Seq<'a>,
    pub b: Seq<'a>,

    pub params: &'a AstarPa2<V, H>,

    /// The instantiated heuristic to use.
    pub domain: Domain<H::Instance<'a>>,

    /// Hint for the heuristic, cached between `j_range` calls.
    pub hint: <H::Instance<'a> as HeuristicInstance<'a>>::Hint,

    /// The instantiated visualizer to use.
    pub v: V::Instance,
}

impl<V: VisualizerT, H: Heuristic> Aligner for AstarPa2<V, H> {
    fn align(&mut self, a: Seq, b: Seq) -> (Cost, Option<Cigar>) {
        self.cost_or_align(a, b, self.trace)
    }
}

impl<'a, V: VisualizerT, H: Heuristic> Drop for AstarPa2Instance<'a, V, H> {
    fn drop(&mut self) {
        if DEBUG {
            if let Astar(h) = &mut self.domain {
                eprintln!("h0 end: {}", h.h(Pos(0, 0)));
            }
        }
    }
}

impl<'a, V: VisualizerT, H: Heuristic> AstarPa2Instance<'a, V, H> {
    /// The range of rows `j` to consider for columns `i_range.0 .. i_range.1`, when the cost is bounded by `f_bound`.
    ///
    /// For A*, this also returns the range of rows in column `i_range.0` that are 'fixed', ie have `f <= f_max`.
    /// TODO: We could actually also return such a range in non-A* cases.
    ///
    /// `i_range`: `[start, end)` range of characters of `a` to process. Ends with column `end` of the DP matrix.
    /// Pass `-1..0` for the range of the first column. `prev` is not used.
    /// Pass `i..i+1` to move 1 block, with `prev` the block for column `i`,
    /// Pass `i..i+W` to compute a block of `W` columns `i .. i+W`.
    ///
    ///
    /// `old_range`: The old j_range at the end of the current interval, to ensure it only grows.
    ///
    /// ALG: We must continue from the old_j_range to ensure things work well after pruning:
    /// Pruning is only allowed if we guarantee that the range never shrinks,
    /// and it can happen that we 'run out' of `f(u) <= f_max` states inside the
    /// `old_range`, while extending the `old_range` from the bottom could grow
    /// more.
    fn j_range(
        &mut self,
        i_range: IRange,
        f_max: Option<Cost>,
        prev: &Block,
        old_range: Option<JRange>,
    ) -> JRange {
        // Without a bound on the distance, we can only return the full range.
        let Some(f_max) = f_max else {
            return JRange(0, self.b.len() as I);
        };

        // Inclusive start column of the new block.
        let is = i_range.0;
        // Inclusive end column of the new block.
        let ie = i_range.1;

        let unit_cost = AffineCost::unit();

        let mut range = match &self.domain {
            Full => JRange(0, self.b.len() as I),
            GapStart => {
                // range: the max number of diagonals we can move up/down from the start with cost f.
                JRange(
                    is + 1 + -(unit_cost.max_del_for_cost(f_max) as I),
                    ie + unit_cost.max_ins_for_cost(f_max) as I,
                )
            }
            GapGap => {
                let d = self.b.len() as I - self.a.len() as I;
                // We subtract the cost needed to bridge the gap from the start to the end.
                let s = f_max - unit_cost.gap_cost(Pos(0, 0), Pos::target(&self.a, &self.b));
                // Each extra diagonal costs one insertion and one deletion.
                let extra_diagonals = s / (unit_cost.min_ins_extend + unit_cost.min_del_extend);
                // NOTE: The range could be reduced slightly further by considering gap open costs.
                JRange(
                    is + 1 + min(d, 0) - extra_diagonals as I,
                    ie + max(d, 0) + extra_diagonals as I,
                )
            }
            Astar(h) => {
                // TODO FIXME Return already-rounded jrange. More precision isn't needed, and this will save some time.

                // Get the range of rows with fixed states `f(u) <= f_max`.
                let JRange(mut fixed_start, mut fixed_end) = prev
                    .fixed_j_range
                    .expect("With A* Domain, fixed_j_range should always be set.");
                if DEBUG {
                    eprintln!("j_range for {i_range:?}\t\told {old_range:?}\t\t fixed @ {is}\t {fixed_start}..{fixed_end}");
                }
                assert!(fixed_start <= fixed_end, "Fixed range must not be empty");

                // Make sure we do not leave out states computed in previous iterations.
                // The domain may never shrink!
                if let Some(old_range) = old_range {
                    fixed_start = min(fixed_start, old_range.0);
                    fixed_end = max(fixed_end, old_range.1);
                }

                // The start of the j_range we will compute for this block is the `fixed_start` of the previous column.
                // The end of the j_range is extrapolated from `fixed_end`.

                // `u` is the bottom most fixed state in prev col.
                let u = Pos(is, fixed_end);
                let gu = if is < 0 { 0 } else { prev.index(fixed_end) };
                // in the end, `v` will be the bottom most state in column
                // i_range.1 that could possibly have `f(v) <= f_max`.
                let mut v = u;

                // Wrapper to use h with hint.
                let mut h = |pos| {
                    let (h, new_hint) = h.h_with_hint(pos, self.hint);
                    self.hint = new_hint;
                    self.v.h_call(pos);
                    h
                };
                // A lower bound of `f` values estimated from `gu`, valid for states `v` below the diagonal of `u`.
                let mut f = |v: Pos| {
                    assert!(v.1 - u.1 >= v.0 - u.0);
                    gu + unit_cost.extend_cost(u, v) + h(v)
                };

                // Extend `v` diagonally one column at a time towards `ie`.
                // In each column, find the lowest `v` such that
                // `f(v) = g(v) + h(v) <= gu + extend_cost(u, v) + h(v) <= s`.
                //
                // NOTE: We can not directly go to the last column, since
                // the optimal path could then 'escape' through the bottom.
                // Without further reasoning, we must evaluate `h` at least
                // once per column.

                if !self.params.sparse_h {
                    while v.0 < ie {
                        // Extend diagonally.
                        v += Pos(1, 1);

                        // Extend down while cell below is in-reach.
                        v.1 += 1;
                        while v.1 <= self.b.len() as I && f(v) <= f_max {
                            v.1 += 1;
                        }
                        v.1 -= 1;
                    }
                } else {
                    // FIXME: Can we drop this??
                    v += Pos(1, 1);
                    // ALG:
                    // First go down by block width, anticipating that extending diagonally will not increase f.
                    // (This is important; f doesn't work for `v` above the diagonal of `u`.)
                    // Then repeat:
                    // - Go right until in-scope using exponential steps.
                    // - Go down until out-of-scope using steps of size 8.
                    // Finally, go up to in-scope.
                    // NOTE: We start with a small additional buffer to prevent doing v.1 += 1 in the loop below.
                    v.1 += self.params.block_width + 8;
                    v.1 = min(v.1, self.b.len() as I);
                    while v.0 <= ie && v.1 < self.b.len() as I {
                        let fv = f(v);
                        if fv <= f_max {
                            v.1 += 8;
                        } else {
                            // By consistency of `f`, it can only change value by at most `2` per step in the unit cost setting.
                            // When `f(v) > f_max`, this means we have to make at least `ceil((fv - f_max)/2)` steps to possibly get at a cell with `f(v) <= f_max`.
                            v.0 += (fv - f_max).div_ceil(2 * unit_cost.min_del_extend);
                        }
                    }
                    v.0 = ie;
                    loop {
                        // Stop in the edge case where `f(v)` would be invalid (`v.1<0`)
                        // or when the bottom of the grid was reached, in which
                        // case `v` may not be below the diagonal of `u`, and
                        // simply computing everything won't loose much anyway.
                        if v.1 < 0 || v.1 == self.b.len() as I {
                            break;
                        }
                        let fv = f(v);
                        if fv <= f_max {
                            break;
                        } else {
                            v.1 -= (fv - f_max).div_ceil(2 * unit_cost.min_ins_extend);
                            // Don't go above the diagonal.
                            // This could happen after pruning we if don't check explicitly.
                            if v.1 < v.0 - u.0 + u.1 {
                                v.1 = v.0 - u.0 + u.1;
                                break;
                            }
                        }
                    }
                }
                JRange(fixed_start, v.1)
            }
        };
        // Size at least old_range.
        if let Some(old_range) = old_range {
            range = range.union(old_range);
        }
        // crop
        range.intersection(JRange(0, self.b.len() as I))
    }

    /// Compute the j_range of `block` `i` with `f(u) <= f_max`.
    /// BUG: This should take into account potential non-consistency of `h`.
    /// In particular, with inexact matches, we can only fix states with `f(u) <= f_max - r`.
    fn fixed_j_range(&mut self, i: I, f_max: Option<Cost>, block: &Block) -> Option<JRange> {
        let Astar(h) = &self.domain else {
            return None;
        };
        let Some(f_max) = f_max else {
            return None;
        };

        // Wrapper to use h with hint.
        let mut h = |pos| {
            let (h, new_hint) = h.h_with_hint(pos, self.hint);
            self.hint = new_hint;
            h
        };

        // Compute values at the end of each lane.
        let mut f = |j| block.index(j) + h(Pos(i, j));

        // Start: increment the start of the range until f<=f_max is satisfied.
        // End: decrement the end of the range until f<=f_max is satisfied.
        //
        // ALG: Sparse h-calls:
        // Set u = (i, start), and compute f(u).
        // For v = (i, j), (j>start) we have
        // - g(v) >= g(u) - (j - start), by triangle inequality
        // - h(u) <= (j - start) + h(v), by 'column-wise-consistency'
        // => f(u) = g(u) + h(u) <= g(v) + h(v) + 2*(j - start) = f(v) + 2*(j - start)
        // => f(v) >= f(u) - 2*(j - start)
        // We want f(v) <= f_max, so we can stop when f(u) - 2*(j - start) <= f_max, ie
        // j >= start + (f(u) - f_max) / 2
        // Thus, both for increasing `start` and decreasing `end`, we can jump ahead if the difference is too large.
        // TODO: It may be sufficient to only compute this with rounded-to-64 precision.
        let mut start = block.j_range.0;
        let mut end = block.j_range.1;
        while start <= end {
            let f = f(start);
            if f <= f_max {
                break;
            }
            start += if self.params.sparse_h {
                (f - f_max).div_ceil(2 * AffineCost::unit().min_ins_extend)
            } else {
                1
            };
        }

        while end >= start {
            let f = f(end);
            if f <= f_max {
                break;
            }
            end -= if self.params.sparse_h {
                (f - f_max).div_ceil(2 * AffineCost::unit().min_ins_extend)
            } else {
                1
            };
        }
        if DEBUG {
            eprintln!("initial fixed_j_range for {i} {fixed_j_range:?}");
            eprintln!("prev    fixed_j_range for {i} {:?}", block.fixed_j_range);
        }
        Some(JRange(start, end))
    }

    /// Test whether the cost is at most s.
    /// Returns None if no path was found.
    /// It may happen that a path is found, but the cost is larger than s.
    /// In this case no cigar is returned.
    pub fn align_for_bounded_dist(
        &mut self,
        f_max: Option<Cost>,
        trace: bool,
        blocks: Option<&mut Blocks>,
    ) -> Option<(Cost, Option<Cigar>)> {
        // Update contours for any pending prunes.
        if self.params.prune
            && let Astar(h) = &mut self.domain
        {
            h.update_contours(Pos(0, 0));
            if DEBUG {
                eprintln!("\nTEST DIST {} h0 {}\n", f_max.unwrap_or(0), h.h(Pos(0, 0)));
            }
        } else {
            if DEBUG {
                eprintln!("\nTEST DIST {}\n", f_max.unwrap_or(0));
            }
        }

        // Make a local block variable if not passed in.
        let mut local_blocks = if blocks.is_none() {
            Some(self.params.block.new(trace, self.a, self.b))
        } else {
            None
        };
        let blocks = if let Some(blocks) = blocks {
            blocks
        } else {
            local_blocks.as_mut().unwrap()
        };

        assert!(f_max.unwrap_or(0) >= 0);

        // Set up initial block for column 0.
        let initial_j_range = self.j_range(
            IRange::first_col(),
            f_max,
            &Block {
                fixed_j_range: Some(JRange(-1, -1)),
                ..Block::default()
            },
            blocks.next_block_j_range(),
        );

        // If 0 is not included in the initial range, no path can be found.
        // This can happen for e.g. the GapGap heuristic when the threshold is too small.
        // Note that the range never shrinks, so even after pruning it should still start at 0.
        if initial_j_range.is_empty() || initial_j_range.0 > 0 {
            return None;
        }

        blocks.init(initial_j_range);
        blocks.set_last_block_fixed_j_range(Some(initial_j_range));

        self.v.expand_block(
            Pos(0, 0),
            Pos(1, blocks.last_block().j_range.len()),
            0,
            f_max.unwrap_or(0),
            self.domain.h(),
        );

        let mut all_blocks_reused = true;

        for i in (0..self.a.len() as I).step_by(self.params.block_width as _) {
            // The i_range of the new block.
            let i_range = IRange(i, min(i + self.params.block_width, self.a.len() as I));
            // The j_range of the new block.
            let j_range = self.j_range(
                i_range,
                f_max,
                // The last block is needed to query `g(u)` in the last column.
                blocks.last_block(),
                // An existing `j_range` for a previous iteration may be
                // present, in which case we ensure the `j_range` does not
                // shrink.
                blocks.next_block_j_range(),
            );

            if j_range.is_empty() {
                assert!(blocks.next_block_j_range().is_none());
                self.v.new_layer(self.domain.h());
                return None;
            }

            // If the new `j_range` is the same as the old one, and all previous
            // blocks were reused, we can also reuse this new block.
            let mut reuse = false;
            if blocks.next_block_j_range() == Some(j_range) && all_blocks_reused {
                reuse = true;
            }
            all_blocks_reused &= reuse;

            // Store before appending a new block.
            let prev_fixed_j_range = blocks.last_block().fixed_j_range;

            // Reuse or compute the next block.
            if reuse {
                blocks.reuse_next_block(i_range, j_range);
            } else {
                blocks.compute_next_block(i_range, j_range, &mut self.v);
                if self.params.doubling == DoublingType::None {
                    self.v.new_layer(self.domain.h());
                }
            }

            // Compute the new range of fixed states.
            let next_fixed_j_range = self.fixed_j_range(i_range.1, f_max, blocks.last_block());

            // If there are no fixed states, break.
            if next_fixed_j_range.is_some_and(|r| r.is_empty()) {
                if DEBUG {
                    eprintln!("fixed_j_range is empty! Increasing f_max!");
                }
                self.v.new_layer(self.domain.h());
                return None;
            }
            blocks.set_last_block_fixed_j_range(next_fixed_j_range);

            // Prune matches in the intersection of the previous and next fixed range.
            if self.params.prune
                && let Astar(h) = &mut self.domain
            {
                let intersection =
                    JRange::intersection(prev_fixed_j_range.unwrap(), next_fixed_j_range.unwrap());
                if !intersection.is_empty() {
                    h.prune_block(i_range.0..i_range.1, intersection.0..intersection.1);
                }
            }
        }

        self.v.new_layer(self.domain.h());

        let Some(dist) = blocks.last_block().get(self.b.len() as I) else {
            return None;
        };

        // If dist is at most the assumed bound, do a traceback.
        if trace && dist <= f_max.unwrap_or(I::MAX) {
            let cigar = blocks.trace(
                self.a,
                self.b,
                Pos(0, 0),
                Pos(self.a.len() as I, self.b.len() as I),
                &mut self.v,
            );
            Some((dist, Some(cigar)))
        } else {
            // NOTE: A distance is always returned, even if it is larger than
            // the assumed bound, since this can be used as an upper bound on the
            // distance in further iterations.
            Some((dist, None))
        }
    }
}
