use std::mem::swap;

use clap::{Parser, ValueEnum};
use itertools::Itertools;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::aligners::Sequence;

#[derive(ValueEnum, Default, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorModel {
    #[default]
    Uniform,
    /// Make a single gap (insertion or deletion) of size e*n.
    Gap,
    /// Delete a region of size e*n and insert a region of size e*n.
    Move,
    /// Takes a region of size e*n and insert it
    Insert,
    /// Apply e/2 noise and an insertion of e/2.
    NoisyInsert,
    /// Apply e/2 noise and a deletion of e/2.
    NoisyDelete,
    /// Takes a region of size e*n/2 and inserts it twice in a row next to
    /// each other
    Doubleinsert,
    /// Construct the sequence of n/pattern_length repeating subsequences for sequence A
    /// and adds e*n mutations for sequence B
    Repeat,
    /// Construct the sequence of n/pattern_length repeating subsequences for sequence and adds
    /// e*n mutations for sequence A, and then adds
    /// e*n mutations for sequence B
    MutatedRepeat,
    /// Construct the sequence of n/pattern_length repeating subsequences for sequence and adds
    /// e*n/2 mutations for sequences A and B individually
    DoubleMutatedRepeat,
}

#[derive(Parser, Clone, Serialize, Deserialize)]
pub struct GenerateArgs {
    /// The number of sequence pairs to generate
    #[clap(short = 'x', long, default_value_t = 1, display_order = 2)]
    pub cnt: usize,

    /// Length of generated sequences
    ///
    /// Input sequences from files are also cropped to this length, if set.
    #[clap(short = 'n', long, display_order = 3)]
    pub length: Option<usize>,

    /// The length of b for the case DoubleMutatedRepeat
    #[clap(short, hide_short_help = true)]
    pub m: Option<usize>,

    /// Input error rate
    ///
    /// This is used both to generate input sequences with the given induced
    /// error rate, and to choose values for parameters r and k
    #[clap(short, long, display_order = 4)]
    pub error_rate: Option<f32>,

    #[clap(
        long,
        value_enum,
        default_value_t,
        value_name = "MODEL",
        hide_short_help = true
    )]
    pub error_model: ErrorModel,

    /// Seed to initialize RNG for reproducability
    #[clap(long)]
    pub seed: Option<u64>,

    /// The length of a pattern
    #[clap(long, default_value_t = 0, hide_short_help = true)]
    pub pattern_length: usize,
}

impl GenerateArgs {
    pub fn to_generate_options(&self) -> GenerateOptions {
        GenerateOptions {
            length: self.length.unwrap(),
            error_rate: self.error_rate.unwrap(),
            error_model: self.error_model,
            pattern_length: self.pattern_length,
            m: self.m,
        }
    }
}

pub struct GenerateOptions {
    pub length: usize,
    pub error_rate: f32,
    pub error_model: ErrorModel,
    pub pattern_length: usize,
    pub m: Option<usize>,
}

const ALPH: [char; 4] = ['A', 'C', 'G', 'T'];

enum Mutation {
    // Replace char at pos.
    Substitution(usize, u8),
    // Insert char before pos.
    Insertion(usize, u8),
    // Delete char at pos.
    Deletion(usize),
}

fn rand_char(rng: &mut impl Rng) -> u8 {
    ALPH[rng.gen_range(0..4)] as u8
}

fn random_mutation(len_b: usize, rng: &mut impl Rng) -> Mutation {
    // Substitution / insertion / deletion all with equal probability.
    // For length 0 sequences, only generate insertions.
    match if len_b == 0 {
        1
    } else {
        rng.gen_range(0..3usize)
    } {
        0 => Mutation::Substitution(rng.gen_range(0..len_b), rand_char(rng)),
        1 => Mutation::Insertion(rng.gen_range(0..len_b + 1), rand_char(rng)),
        2 => Mutation::Deletion(rng.gen_range(0..len_b)),
        _ => unreachable!(),
    }
}

pub fn generate_pair(opt: &GenerateOptions, rng: &mut impl Rng) -> (Sequence, Sequence) {
    let mut a = (0..opt.length).map(|_| rand_char(rng)).collect_vec();
    let num_mutations = (opt.error_rate * opt.length as f32).ceil() as usize;
    let mut b = ropey::Rope::from_str(std::str::from_utf8(&a).unwrap());
    'exit: {
        match opt.error_model {
            ErrorModel::Uniform => {
                for _ in 0..num_mutations {
                    make_mutation(&mut b, rng);
                }
            }
            ErrorModel::Gap => {
                if rng.gen_bool(0.5) {
                    // deletion
                    let start = rng.gen_range(0..=b.len_bytes() - num_mutations);
                    b.remove(start..start + num_mutations);
                } else {
                    // insertion
                    let start = rng.gen_range(0..=b.len_bytes());
                    let text = (0..num_mutations).map(|_| rand_char(rng)).collect_vec();
                    b.insert(start, std::str::from_utf8(&text).unwrap());
                }
            }
            ErrorModel::Move => {
                // deletion
                let start = rng.gen_range(0..=b.len_bytes() - num_mutations);
                let piece = b.slice(start..start + num_mutations).to_string();
                b.remove(start..start + num_mutations);
                // insertion
                let start = rng.gen_range(0..=b.len_bytes());
                b.insert(start, piece.as_str());
            }
            ErrorModel::Insert => {
                let start = rng.gen_range(0..b.len_bytes() - num_mutations);
                let piece = b.slice(start..start + num_mutations).to_string();
                b.insert(start, piece.as_str());
            }
            ErrorModel::NoisyInsert | ErrorModel::NoisyDelete => {
                for _ in 0..num_mutations / 2 {
                    make_mutation(&mut b, rng);
                }
                let start = rng.gen_range(0..=b.len_bytes());
                let piece =
                    String::from_utf8((0..num_mutations / 2).map(|_| rand_char(rng)).collect_vec())
                        .unwrap();
                b.insert(start, piece.as_str());
            }
            ErrorModel::Doubleinsert => {
                let start = rng.gen_range(0..=b.len_bytes() - num_mutations);
                let piece = b.slice(start..start + num_mutations / 2).to_string();
                b.insert(start, piece.as_str());
                b.insert(start + piece.len(), piece.as_str());
            }
            ErrorModel::Repeat => {
                if opt.length == 0 {
                    break 'exit;
                }
                let len = if opt.pattern_length != 0 {
                    opt.pattern_length
                } else {
                    rng.gen_range(1..=(opt.length as f32).sqrt() as usize)
                };
                let pattern = ropey::Rope::from_str(
                    std::str::from_utf8(&(0..len).map(|_| rand_char(rng)).collect_vec()).unwrap(),
                );
                a = Vec::new();
                // A is n/pattern_length copies of the pattern
                for _ in 0..opt.length / len {
                    a.append(&mut pattern.to_string().into_bytes());
                }
                b = ropey::Rope::from_str(std::str::from_utf8(&a).unwrap());
                for _ in 0..(opt.length as f32 * opt.error_rate) as usize {
                    make_mutation(&mut b, rng);
                }
            }
            ErrorModel::MutatedRepeat => {
                if opt.length == 0 {
                    break 'exit;
                }
                let len = if opt.pattern_length != 0 {
                    opt.pattern_length
                } else {
                    rng.gen_range(1..=(opt.length as f32).sqrt() as usize)
                };
                let pattern = ropey::Rope::from_str(
                    std::str::from_utf8(&(0..len).map(|_| rand_char(rng)).collect_vec()).unwrap(),
                );
                let mut a_rope = ropey::Rope::new();
                // fill a
                for _ in 0..opt.length / len {
                    a_rope.append(pattern.clone());
                }
                // Apply n*e mutations to A
                for _ in 0..(opt.length as f32 * opt.error_rate) as usize {
                    make_mutation(&mut a_rope, rng);
                }
                b = a_rope.clone();
                // Apply n*e mutations to B
                for _ in 0..(opt.length as f32 * opt.error_rate) as usize {
                    make_mutation(&mut b, rng);
                }
                a = a_rope.to_string().into_bytes();
            }
            ErrorModel::DoubleMutatedRepeat => {
                if opt.length == 0 {
                    break 'exit;
                }
                let len = if opt.pattern_length != 0 {
                    opt.pattern_length
                } else {
                    rng.gen_range(1..=(opt.length as f32).sqrt() as usize)
                };
                let pattern = ropey::Rope::from_str(
                    std::str::from_utf8(&(0..len).map(|_| rand_char(rng)).collect_vec()).unwrap(),
                );
                a = Vec::new();
                // fill a
                for _ in 0..opt.length / len {
                    a.append(&mut pattern.to_string().into_bytes());
                }
                let mut a_rope = ropey::Rope::from_str(std::str::from_utf8(&a).unwrap());
                for _ in 0..(opt.length as f32 * opt.error_rate / 2.) as usize {
                    make_mutation(&mut a_rope, rng);
                }
                b = ropey::Rope::new();
                // fill b
                for _ in 0..opt.m.unwrap_or(opt.length) / len {
                    b.append(pattern.clone());
                }
                for _ in 0..(opt.m.unwrap_or(opt.length) as f32 * opt.error_rate / 2.) as usize {
                    make_mutation(&mut b, rng);
                }
                a = a_rope.to_string().into_bytes();
            }
        }
    }
    let mut b = b.to_string().into_bytes();
    if opt.error_model == ErrorModel::NoisyDelete {
        swap(&mut a, &mut b);
    }
    (a, b)
}

fn make_mutation(b: &mut ropey::Rope, rng: &mut impl Rng) {
    let m = random_mutation(b.len_bytes(), rng);
    match m {
        Mutation::Substitution(i, c) => {
            b.remove(i..=i);
            b.insert(i, std::str::from_utf8(&[c]).unwrap());
        }
        Mutation::Insertion(i, c) => b.insert(i, std::str::from_utf8(&[c]).unwrap()),
        Mutation::Deletion(i) => {
            b.remove(i..=i);
        }
    }
}

// For quick testing
pub fn setup_with_seed(n: usize, e: f32, seed: u64) -> (Sequence, Sequence) {
    setup_sequences_with_seed(seed, n, e)
}

pub fn setup_sequences(n: usize, e: f32) -> (Sequence, Sequence) {
    setup_sequences_with_seed(31415, n, e)
}
pub fn setup_sequences_with_seed(seed: u64, n: usize, e: f32) -> (Sequence, Sequence) {
    setup_sequences_with_seed_and_model(seed, n, e, ErrorModel::Uniform)
}
pub fn setup_sequences_with_seed_and_model(
    seed: u64,
    n: usize,
    e: f32,
    error_model: ErrorModel,
) -> (Sequence, Sequence) {
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed as u64);
    generate_pair(
        &GenerateOptions {
            length: n,
            error_rate: e,
            error_model,
            pattern_length: 0,
            m: None,
        },
        &mut rng,
    )
}

pub fn setup(n: usize, e: f32) -> (Sequence, Sequence) {
    setup_with_seed(n, e, 31415)
}

#[cfg(test)]
mod test {
    use rand::SeedableRng;

    use super::*;

    // Baseline implementation using quadratic implementation.
    fn generate_pair_quadratic(n: usize, e: f32, rng: &mut impl Rng) -> (Sequence, Sequence) {
        let a = (0..n).map(|_| rand_char(rng)).collect_vec();
        let num_mutations = (e * n as f32).ceil() as usize;
        let mut b = a.clone();
        for _ in 0..num_mutations {
            let m = random_mutation(b.len(), rng);
            match m {
                Mutation::Substitution(i, c) => {
                    b[i] = c;
                }
                Mutation::Insertion(i, c) => b.insert(i, c),
                Mutation::Deletion(i) => {
                    b.remove(i);
                }
            }
        }
        (a, b)
    }

    #[test]
    fn test_rope() {
        let mut rng_1 = rand_chacha::ChaCha8Rng::seed_from_u64(1234);
        let mut rng_2 = rand_chacha::ChaCha8Rng::seed_from_u64(1234);

        for n in [10, 100, 1000] {
            for e in [0.01, 0.1, 0.5, 1.0] {
                let p1 = generate_pair(
                    &GenerateOptions {
                        length: n,
                        error_rate: e,
                        error_model: ErrorModel::Uniform,
                        pattern_length: 0,
                        m: None,
                    },
                    &mut rng_1,
                );
                let p2 = generate_pair_quadratic(n, e, &mut rng_2);
                assert_eq!(p1, p2);
            }
        }
    }
}
