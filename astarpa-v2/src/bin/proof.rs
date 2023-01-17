#![feature(let_chains)]

use astarpa::{
    cli::{heuristic_params::AlgorithmArgs, input::Input, visualizer::VisualizerArgs},
    prelude::*,
};
use clap::Parser;
use cli::heuristic_params::HeuristicArgs;
use itertools::Itertools;
use std::{ops::ControlFlow, path::PathBuf, time::Duration};

#[derive(Parser)]
#[clap(author, about)]
struct Cli {
    #[clap(flatten)]
    input: Input,

    /// Where to write optional statistics.
    #[clap(short, long, value_parser = clap::value_parser!(PathBuf))]
    output: Option<PathBuf>,

    /// Parameters and settings for the algorithm.
    #[clap(flatten)]
    algorithm: AlgorithmArgs,

    /// Parameters and settings for the heuristic.
    #[clap(flatten)]
    heuristic: HeuristicArgs,

    /// Parameters and settings for the visualizer.
    #[clap(flatten)]
    visualizer: VisualizerArgs,

    /// Print less. Pass twice for summary line only.
    ///
    /// Do not print a new line per alignment, but instead overwrite the previous one.
    /// Pass twice to only print a summary line and avoid all terminal clutter, e.g. for benchmarking.
    #[clap(short, long, action = clap::ArgAction::Count )]
    silent: u8,

    /// Stop aligning new pairs after this timeout.
    #[clap(long, value_parser =  parse_duration::parse)]
    timeout: Option<Duration>,
}

fn main() {
    let args = Cli::parse();

    // Read the input
    let mut avg_result = AstarStats::default();
    let start = instant::Instant::now();

    args.input.process_input_pairs(|a: Seq, b: Seq| {
        // let h = PathHeuristic { h: self.h };
        // FIXME: WRAP IN PATH_HEURISTIC.
        let r = aligners::astar::AstarPAParams {
            diagonal_transition: args.algorithm.dt,
            heuristic: args.heuristic.clone(),
        }
        .aligner_with_visualizer(&args.visualizer)
        .align_with_stats(a, b)
        .1;

        // Record and print stats.
        if args.silent <= 1 {
            print!("\r");
            if args.silent == 0 {
                r.print();
            }
        }
        avg_result += r;
        if args.silent <= 1 {
            avg_result.print_no_newline();
        }

        if let Some(d) = args.timeout && start.elapsed() > d {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });

    if avg_result.sample_size > 0 {
        print!("\r");
        avg_result.print();

        if let Some(output) = args.output {
            let (header, vals) = avg_result.values();

            std::fs::write(
                output,
                format!(
                    "{}\n{}\n",
                    header.iter().map(|x| x.trim()).join("\t"),
                    vals.iter().map(|x| x.trim()).join("\t")
                ),
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod test {
    use super::Cli;

    #[test]
    fn cli_test() {
        <Cli as clap::CommandFactory>::command().debug_assert();
    }
}
