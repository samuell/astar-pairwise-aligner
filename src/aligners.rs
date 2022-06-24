//! This module contains implementations of other alignment algorithms.

use std::cmp::max;

use crate::prelude::{Cost, CostModel, Pos};

use self::{cigar::Cigar, nw::Path};

pub mod cigar;
pub mod diagonal_transition;
pub mod front;
pub mod nw;
pub mod nw_lib;

#[cfg(test)]
mod tests;

/// An owned sequence.
pub type Sequence = Vec<u8>;
/// A sequence slice.
pub type Seq<'a> = &'a [u8];

/// A visualizer can be used to visualize progress of an implementation.
pub trait VisualizerT {
    fn explore(&mut self, _pos: Pos) {}
    fn expand(&mut self, _pos: Pos) {}
}

/// A trivial visualizer that does not do anything.
struct NoVisualizer;
impl VisualizerT for NoVisualizer {}

/// An aligner is a type that supports aligning sequences using some algorithm.
/// It should implement the most general of the methods below.
/// The cost-only variant can sometimes be implemented using less memory.
///
/// There is one function for each cost model:
/// - LinearCost
/// - AffineCost
///
/// The output can be:
/// - cost only
/// - cost and alignment
/// - cost, alignment, and a visualization.
///
/// Note that insertions are when `b` has more characters than `a`, and deletions are when `b` has less characters than `a`.
pub trait Aligner {
    type CostModel: CostModel;

    /// Returns the cost model used by the aligner.
    fn cost_model(&self) -> &Self::CostModel;

    /// Finds the cost of aligning `a` and `b`.
    /// Uses the visualizer to record progress.
    fn cost(&mut self, a: Seq, b: Seq) -> Cost;

    /// Finds an alignments (path/Cigar) of sequences `a` and `b`.
    /// Uses the visualizer to record progress.
    fn align(&mut self, a: Seq, b: Seq) -> (Cost, Path, Cigar);

    /// Finds the cost of aligning `a` and `b`, assuming the cost of the alignment is at most s.
    /// The returned cost may be `None` in case aligning with cost at most `s` is not possible.
    /// The returned cost may be larger than `s` when a path was found, even
    /// though this may not be the optimal cost.
    fn cost_for_bounded_dist(&mut self, _a: Seq, _b: Seq, _s_bound: Cost) -> Option<Cost>;

    /// Finds an alignments (path/Cigar) of sequences `a` and `b`, assuming the
    /// cost of the alignment is at most s.
    /// The returned cost may be `None` in case aligning with cost at most `s` is not possible.
    /// The returned cost may be larger than `s` when a path was found, even
    /// though this may not be the optimal cost.
    fn align_for_bounded_dist(
        &mut self,
        _a: Seq,
        _b: Seq,
        _s_bound: Cost,
    ) -> Option<(Cost, Path, Cigar)>;

    /// Find the cost using exponential search based on `cost_assuming_bounded_dist`.
    /// TODO: Allow customizing the growth factor.
    fn cost_exponential_search(&mut self, a: Seq, b: Seq) -> Cost {
        let mut s: Cost = self
            .cost_model()
            .gap_cost(Pos(0, 0), Pos::from_lengths(a, b));
        // TODO: Fix the potential infinite loop here.
        loop {
            if let Some(cost) = self.cost_for_bounded_dist(a, b, s) && cost <= s{
                return cost;
            }
            s = max(2 * s, 1);
        }
    }

    /// Find the alignment using exponential search based on `align_assuming_bounded_dist`.
    /// TODO: Allow customizing the growth factor.
    fn align_exponential_search(&mut self, a: Seq, b: Seq) -> (Cost, Path, Cigar) {
        let mut s: Cost = self
            .cost_model()
            .gap_cost(Pos(0, 0), Pos::from_lengths(a, b));
        // TODO: Fix the potential infinite loop here.
        loop {
            if let Some(tuple@(cost, _, _)) = self.align_for_bounded_dist(a, b, s) && cost <= s{
                return tuple;
            }
            s = max(2 * s, 1);
        }
    }
}
