use crate::visualizer::*;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Parser, Serialize, Deserialize)]
#[clap(help_heading = "VISUALIZER")]
pub struct VisualizerArgs {
    /// Run the interactive visualizer. See --help for controls. [default: all]
    ///
    /// x: exit
    /// p: pause / unpaues
    /// esc, space: next frame
    /// f: faster
    /// s: slower
    /// q: jump to last frame, or exit when already on last frame
    #[clap(short = 'v', long, display_order = 1, value_enum, value_name = "WHEN", default_value_t = When::None)]
    pub visualize: When,

    /// Visualizer style.
    #[clap(long, default_value_t, value_enum, display_order = 2)]
    pub style: VisualizerStyle,

    /// Start paused.
    #[clap(short, long, display_order = 3)]
    pub pause: bool,

    /// Which frames to save.
    #[clap(long, display_order = 3, value_enum, value_name = "WHEN", default_value_t = When::None)]
    pub save: When,

    /// Show or save only each Nth frame.
    #[clap(long, display_order = 3)]
    pub each: Option<usize>,

    /// Where to save. Implies --save [last].
    #[clap(long, display_order = 4, value_name = "PATH", parse(from_os_str))]
    pub save_path: Option<PathBuf>,

    /// The size in pixels of each cell.
    /// By default, chosen to give a canvas of height 500.
    #[clap(long, display_order = 10, hide_short_help = true)]
    pub cell_size: Option<u32>,

    /// Number of states per cell.
    /// By default, chosen to give a canvas of height 500.
    #[clap(long, display_order = 10, hide_short_help = true)]
    pub downscaler: Option<u32>,

    /// When set, draw newer expanded states on top. Useful for divide & conquer approaches.
    #[clap(long, display_order = 10, hide_short_help = true)]
    pub new_on_top: bool,

    /// Enable drawing the tree.
    #[clap(long, display_order = 10, hide_short_help = true)]
    pub draw_tree: bool,

    /// Disable drawing the tree.
    #[clap(long, display_order = 10, hide_short_help = true)]
    pub no_draw_tree: bool,
}

pub trait VisualizerRunner {
    type R;
    fn call<V: VisualizerT>(&self, v: V) -> Self::R;
}

pub enum VisualizerType {
    NoVizualizer,
    #[cfg(any(feature = "sdl2", feature = "wasm"))]
    Visualizer(Config),
}

impl VisualizerArgs {
    pub fn make_visualizer(&self) -> VisualizerType {
        #[cfg(not(any(feature = "sdl2", feature = "wasm")))]
        return VisualizerType::NoVizualizer(NoVisualizer);

        #[cfg(any(feature = "sdl2", feature = "wasm"))]
        {
            use crate::canvas::BLACK;

            if self.visualize == When::None && self.save == When::None {
                return VisualizerType::NoVizualizer;
            }

            // Get the default config for the style.
            let mut config = Config::new(self.style);
            config.draw = self.visualize.clone();
            config.save = self.save.clone();
            if config.save != When::None {
                config.save_last = true;
                // In this case, the save_last above is sufficient.
                if config.save == When::Last {
                    config.save = When::None;
                }
                config.filepath = self
                    .save_path
                    .clone()
                    .expect("--save-path must be set when --save is set");
            }
            let update = |when: &mut When| {
                if let Some(step) = self.each {
                    if *when == When::All {
                        *when = When::StepBy(step);
                    }
                    if *when == When::Layers {
                        *when = When::LayersStepBy(step);
                    }
                }
            };
            update(&mut config.draw);
            update(&mut config.save);

            config.paused = self.pause;

            // Apply CLI flag customizations to the style.
            config.cell_size = self.cell_size.unwrap_or(0);
            config.downscaler = self.downscaler.unwrap_or(0);

            config.draw_old_on_top = !self.new_on_top;
            if self.draw_tree {
                config.style.tree = Some(BLACK);
            }
            if self.no_draw_tree {
                config.style.tree = None;
            }

            #[cfg(feature = "wasm")]
            {
                config.draw_single_frame = Some(unsafe { crate::wasm::INTERACTION.get() });
            }

            VisualizerType::Visualizer(config)
        }
    }
}
