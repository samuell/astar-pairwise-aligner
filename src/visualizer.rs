//! To turn images into a video, use this:
//!
//! ```sh
//! ffmpeg -framerate 20 -i %d.bmp output.mp4
//! ```
//! or when that gives errors:
//! ```sh
//! ffmpeg -framerate 20 -i %d.bmp -vf "pad=ceil(iw/2)*2:ceil(ih/2)*2" output.mp4
//! ```

use crate::{
    aligners::{cigar::Cigar, cigar::CigarOp, edit_graph::State},
    cost_model::Cost,
    heuristic::{HeuristicInstance, NoCostI},
    prelude::Pos,
};

#[derive(Debug, PartialEq, Default, Clone, Copy, ValueEnum)]
pub enum VisualizerStyle {
    #[default]
    Default,
    Large,
    Detailed,
}

#[derive(Debug, PartialEq, Eq, Clone, ValueEnum)]
pub enum When {
    None,
    Last,
    All,
    Layers,
    // Show/save each Nth frame.
    #[clap(skip)]
    StepBy(usize),
    // Show/save each Nth layer.
    #[clap(skip)]
    LayersStepBy(usize),
    #[clap(skip)]
    Frames(Vec<usize>),
}

#[derive(Debug, PartialEq, Eq, Clone, ValueEnum)]
pub enum HAlign {
    Left,
    Center,
    Right,
}
#[derive(Debug, PartialEq, Eq, Clone, ValueEnum)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

fn make_label(text: &str, val: impl ToString) -> String {
    text.to_string() + &val.to_string()
}

type ParentFn<'a> = Option<&'a dyn Fn(State) -> Option<(State, [Option<CigarOp>; 2])>>;

/// A visualizer can be used to visualize progress of an implementation.
pub trait VisualizerT {
    fn explore(&mut self, pos: Pos, g: Cost, f: Cost) {
        self.explore_with_h::<NoCostI>(pos, g, f, None);
    }
    fn expand(&mut self, pos: Pos, g: Cost, f: Cost) {
        self.expand_with_h::<NoCostI>(pos, g, f, None);
    }
    fn extend(&mut self, pos: Pos, g: Cost, f: Cost) {
        self.extend_with_h::<NoCostI>(pos, g, f, None);
    }
    fn explore_with_h<'a, HI: HeuristicInstance<'a>>(
        &mut self,
        _pos: Pos,
        _g: Cost,
        _f: Cost,
        _h: Option<&HI>,
    ) {
    }
    fn expand_with_h<'a, HI: HeuristicInstance<'a>>(
        &mut self,
        _pos: Pos,
        _g: Cost,
        _f: Cost,
        _h: Option<&HI>,
    ) {
    }
    fn extend_with_h<'a, HI: HeuristicInstance<'a>>(
        &mut self,
        _pos: Pos,
        _g: Cost,
        _f: Cost,
        _h: Option<&HI>,
    ) {
    }

    /// This function should be called after completing each layer
    fn new_layer(&mut self) {
        self.new_layer_with_h::<NoCostI>(None);
    }
    fn new_layer_with_h<'a, HI: HeuristicInstance<'a>>(&mut self, _h: Option<&HI>) {}

    /// This function may be called after the main loop to display final image.
    fn last_frame(&mut self, cigar: Option<&Cigar>) {
        self.last_frame_with_h::<NoCostI>(cigar, None, None);
    }
    fn last_frame_with_tree(&mut self, cigar: Option<&Cigar>, parent: ParentFn) {
        self.last_frame_with_h::<NoCostI>(cigar, parent, None);
    }
    fn last_frame_with_h<'a, HI: HeuristicInstance<'a>>(
        &mut self,
        _cigar: Option<&Cigar>,
        _parent: ParentFn<'_>,
        _h: Option<&HI>,
    ) {
    }
}

/// A trivial visualizer that does not do anything.
pub struct NoVisualizer;
impl VisualizerT for NoVisualizer {}

use clap::ValueEnum;
#[cfg(feature = "sdl2")]
pub use with_sdl2::*;

#[cfg(feature = "sdl2")]
mod with_sdl2 {
    use super::*;
    use crate::{
        aligners::{cigar::Cigar, edit_graph::State, StateT},
        cli::heuristic_params::{comment, AlgorithmArgs, HeuristicArgs},
        cost_model::LinearCost,
        matches::MatchStatus,
        prelude::Seq,
    };
    use itertools::Itertools;
    #[cfg(feature = "sdl2_ttf")]
    use sdl2::ttf::{Font, Sdl2TtfContext};
    use sdl2::{
        event::Event,
        keyboard::Keycode,
        pixels::Color,
        rect::{Point, Rect},
        render::Canvas,
        video::Window,
        Sdl,
    };
    use std::{
        cell::{RefCell, RefMut},
        cmp::{max, min},
        collections::HashMap,
        ops::Range,
        path,
        time::{Duration, Instant},
    };

    #[cfg(feature = "sdl2_ttf")]
    lazy_static! {
        static ref TTF_CONTEXT: Sdl2TtfContext = sdl2::ttf::init().unwrap();
    }

    #[derive(PartialEq, Eq, Clone, Copy)]
    pub enum Type {
        Expanded,
        Explored,
        Extended,
    }
    use Type::*;

    pub struct Visualizer {
        config: Config,

        // Name of the algorithm
        title: Option<String>,
        // Heuristic / algorithm parameters. List of (key, value).
        params: Option<String>,
        // An optional comment explaining the algorithm.
        comment: Option<String>,

        sdl_context: Sdl,
        #[cfg(feature = "sdl2_ttf")]
        font: Font<'static, 'static>,

        canvas: Option<RefCell<Canvas<Window>>>,

        // The size in pixels of the entire canvas.
        canvas_size: (u32, u32),
        // The size in pixels of the NW half of the canvas.
        nw_size: (u32, u32),
        // The size in pixels of the DT half of the canvas.
        dt_size: (u32, u32),

        // The last DP state (a.len(), b.len()).
        target: Pos,

        frame_number: usize,
        layer_number: usize,
        file_number: usize,

        // Type, Pos, g, f
        pub expanded: Vec<(Type, Pos, Cost, Cost)>,
        layer: Option<usize>,
        // Index in expanded where each layer stars.
        expanded_layers: Vec<usize>,
    }

    impl VisualizerT for Visualizer {
        fn explore_with_h<'a, H: HeuristicInstance<'a>>(
            &mut self,
            pos: Pos,
            g: Cost,
            f: Cost,
            h: Option<&H>,
        ) {
            if !(pos <= self.target) {
                return;
            }
            self.expanded.push((Explored, pos, g, f));
            self.draw(false, None, false, h, None);
        }

        fn expand_with_h<'a, H: HeuristicInstance<'a>>(
            &mut self,
            pos: Pos,
            g: Cost,
            f: Cost,
            h: Option<&H>,
        ) {
            if !(pos <= self.target) {
                return;
            }
            self.expanded.push((Expanded, pos, g, f));
            self.draw(false, None, false, h, None);
        }

        fn extend_with_h<'a, H: HeuristicInstance<'a>>(
            &mut self,
            pos: Pos,
            g: Cost,
            f: Cost,
            h: Option<&H>,
        ) {
            if !(pos <= self.target) {
                return;
            }
            self.expanded.push((Extended, pos, g, f));
            self.draw(false, None, false, h, None);
        }

        fn new_layer_with_h<'a, H: HeuristicInstance<'a>>(&mut self, h: Option<&H>) {
            if let Some(layer) = self.layer {
                self.layer = Some(layer + 1);
                self.expanded_layers.push(self.expanded.len());
            }
            self.draw(false, None, true, h, None);
        }

        fn last_frame_with_h<'a, H: HeuristicInstance<'a>>(
            &mut self,
            cigar: Option<&Cigar>,
            parent: ParentFn<'_>,
            h: Option<&H>,
        ) {
            self.draw(true, cigar, false, h, parent);
        }
    }

    #[derive(Clone)]
    pub enum Gradient {
        Fixed(Color),
        Gradient(Range<Color>),
        // 0 <= start < end <= 1
        TurboGradient(Range<f64>),
    }

    impl Gradient {
        fn color(&self, f: f64) -> Color {
            match self {
                Gradient::Fixed(color) => *color,
                Gradient::Gradient(range) => {
                    let frac = |a: u8, b: u8| -> u8 {
                        (a as f64 + f * (b as f64 - a as f64)).ceil() as u8
                    };
                    Color::RGB(
                        frac(range.start.r, range.end.r),
                        frac(range.start.g, range.end.g),
                        frac(range.start.b, range.end.b),
                    )
                }
                Gradient::TurboGradient(range) => {
                    let f = range.start + f * (range.end - range.start);
                    let c = colorgrad::turbo().at(f).to_rgba8();
                    Color::RGBA(c[0], c[1], c[2], c[3])
                }
            }
        }
    }

    #[derive(Clone)]
    pub struct Style {
        pub expanded: Gradient,
        pub explored: Option<Color>,
        pub extended: Option<Color>,
        pub bg_color: Color,
        /// None to disable
        pub path: Option<Color>,
        /// None to draw cells.
        pub path_width: Option<usize>,

        /// None to disable
        pub tree: Option<Color>,
        pub tree_substitution: Option<Color>,
        pub tree_match: Option<Color>,
        pub tree_width: usize,
        pub tree_fr_only: bool,
        pub tree_direction_change: Option<Color>,
        pub tree_affine_open: Option<Color>,

        // Options to draw heuristics
        pub draw_heuristic: bool,
        pub draw_contours: bool,
        pub draw_matches: bool,
        pub heuristic: Gradient,
        pub max_heuristic: Option<u32>,
        pub active_match: Color,
        pub pruned_match: Color,
        pub match_shrink: usize,
        pub match_width: usize,
        pub contour: Color,
    }

    impl When {
        fn is_active(&self, frame: usize, layer: usize, is_last: bool, new_layer: bool) -> bool {
            match &self {
                When::None => false,
                When::Last => is_last,
                When::All => is_last || !new_layer,
                When::Layers => is_last || new_layer,
                When::Frames(v) => v.contains(&frame) || (is_last && v.contains(&usize::MAX)),
                When::StepBy(step) => is_last || frame % step == 0,
                When::LayersStepBy(step) => is_last || (new_layer && layer % step == 0),
            }
        }
    }

    const CANVAS_HEIGHT: u32 = 500;

    #[derive(Clone)]
    pub struct Config {
        /// 0 to infer automatically.
        pub cell_size: u32,
        /// Divide all input coordinates by this for large inputs.
        /// 0 to infer automatically.
        pub downscaler: u32,
        pub filepath: String,
        pub draw: When,
        pub delay: f32,
        pub paused: bool,
        pub save: When,
        pub save_last: bool,
        pub style: Style,
        pub transparent_bmp: bool,
        pub draw_old_on_top: bool,
        pub layer_drawing: bool,
        pub num_layers: Option<usize>,
        pub show_dt: bool,
        pub show_fronts: bool,
    }

    impl Config {
        pub fn new(style: VisualizerStyle) -> Self {
            let mut config = Self {
                cell_size: 8,
                downscaler: 1,
                save: When::None,
                save_last: false,
                filepath: String::from(""),
                draw: When::None,
                delay: 0.1,
                paused: false,
                style: Style {
                    expanded: Gradient::TurboGradient(0.2..0.95),
                    explored: None,
                    extended: None,
                    bg_color: Color::WHITE,
                    path: Some(Color::BLACK),
                    path_width: Some(2),
                    tree: None,
                    tree_substitution: None,
                    tree_match: None,
                    tree_width: 1,
                    tree_fr_only: false,
                    tree_direction_change: None,
                    tree_affine_open: None,
                    draw_heuristic: false,
                    draw_contours: false,
                    draw_matches: false,
                    heuristic: Gradient::Gradient(
                        Color::RGB(250, 250, 250)..Color::RGB(180, 180, 180),
                    ),
                    max_heuristic: None,
                    active_match: Color::BLACK,
                    pruned_match: Color::RED,
                    match_shrink: 2,
                    match_width: 2,
                    contour: Color::GREEN,
                },
                draw_old_on_top: true,
                layer_drawing: false,
                num_layers: None,
                transparent_bmp: true,
                show_dt: true,
                show_fronts: true,
            };

            if style == VisualizerStyle::Large {
                config.transparent_bmp = false;
                config.downscaler = 100;
                config.cell_size = 1;
                config.style.path = None;
                config.style.draw_matches = true;
                config.style.match_width = 1;
                config.style.match_shrink = 0;
                config.style.expanded = Gradient::TurboGradient(0.25..0.90)
            }

            if style == VisualizerStyle::Detailed {
                config.paused = false;
                config.delay = 0.2;
                config.cell_size = 6;
                config.style.bg_color = Color::WHITE;
                config.style.tree = Some(Color::GRAY);
                config.style.expanded = Gradient::Fixed(Color::RGB(130, 179, 102));
                config.style.explored = Some(Color::RGB(0, 102, 204));
                config.style.max_heuristic = Some(10);
                config.style.pruned_match = Color::RED;
                config.style.path = None;
                config.style.match_width = 3;
                config.style.draw_heuristic = true;
                config.style.draw_contours = true;
                config.style.draw_matches = true;
                config.style.contour = Color::BLACK;
                config.draw_old_on_top = true;
                config.layer_drawing = false;
            }

            config
        }
    }

    impl Default for Config {
        fn default() -> Self {
            Config::new(VisualizerStyle::Default)
        }
    }

    impl Visualizer {
        pub fn new(config: Config, a: Seq, b: Seq) -> Self {
            Self::new_with_cli_params(config, a, b, None, None)
        }

        /// This sets the title and parameters based on the CLI arguments.
        pub fn new_with_cli_params(
            mut config: Config,
            a: Seq,
            b: Seq,
            alg: Option<&AlgorithmArgs>,
            heuristic: Option<&HeuristicArgs>,
        ) -> Self {
            let sdl_context = sdl2::init().unwrap();

            // Draw layer numbers
            #[cfg(feature = "sdl2_ttf")]
            let font = TTF_CONTEXT
                .load_font("/usr/share/fonts/TTF/OpenSans-Regular.ttf", 24)
                .unwrap();

            fn new_canvas(
                w: u32,
                h: u32,
                sdl_context: &Sdl,
                title: &str,
            ) -> RefCell<Canvas<Window>> {
                let video_subsystem = sdl_context.video().unwrap();
                video_subsystem.gl_attr().set_double_buffer(true);
                RefCell::new(
                    video_subsystem
                        .window(title, w as u32, h as u32)
                        //.borderless()
                        .build()
                        .unwrap()
                        .into_canvas()
                        .build()
                        .unwrap(),
                )
            }

            // layout:
            //
            // -------------
            // |  NW  | DT |
            // |      |    |
            // -------------
            //
            // NW follows the cell size if given.
            // Otherwise, the cell size and downscaler are chosen to give a height around 500 pixels.
            // The DT window is chosen with the same height, but half the width.

            let grid_width = a.len() as u32 + 1;
            let grid_height = b.len() as u32 + 1;

            if config.cell_size != 0 {
                if config.downscaler == 0 {
                    config.downscaler = 1;
                }
            } else {
                if config.downscaler == 0 {
                    config.downscaler = max(1, grid_height.div_ceil(CANVAS_HEIGHT));
                }
                let ds = config.downscaler;
                config.cell_size = max(1, CANVAS_HEIGHT / (grid_height.div_ceil(ds)));
            }
            let cs = config.cell_size;
            let ds = config.downscaler;
            let nw_size = (grid_width.div_ceil(ds) * cs, grid_height.div_ceil(ds) * cs);
            let dt_size = (nw_size.0 / 2, nw_size.1);
            let canvas_size = (nw_size.0 + dt_size.0, nw_size.1);

            let (params, comment) = if let (Some(alg), Some(h)) = (alg, heuristic) && alg.algorithm.internal(){
                        (Some(h.to_string()), comment(alg, h))
                    } else {
                        (None, None)
                    };
            Visualizer {
                title: alg.map(|alg| alg.to_string()),
                params,
                comment,
                canvas: {
                    (config.draw != When::None || config.save != When::None || config.save_last)
                        .then(|| {
                            new_canvas(canvas_size.0, canvas_size.1, &sdl_context, &config.filepath)
                        })
                },
                #[cfg(feature = "sdl2_ttf")]
                font,
                config: config.clone(),
                expanded: vec![],
                target: Pos::from_lengths(a, b),
                frame_number: 0,
                layer_number: 0,
                file_number: 0,
                layer: if config.layer_drawing { Some(0) } else { None },
                expanded_layers: vec![],
                sdl_context,

                canvas_size,
                nw_size,
                dt_size,
            }
        }

        fn cell_begin(&self, Pos(i, j): Pos) -> Point {
            Point::new(
                (i / self.config.downscaler * self.config.cell_size) as i32,
                (j / self.config.downscaler * self.config.cell_size) as i32,
            )
        }

        fn cell_center(&self, Pos(i, j): Pos) -> Point {
            Point::new(
                (i / self.config.downscaler * self.config.cell_size + self.config.cell_size / 2)
                    as i32,
                (j / self.config.downscaler * self.config.cell_size + self.config.cell_size / 2)
                    as i32,
            )
        }

        fn draw_pixel(&self, canvas: &mut Canvas<Window>, pos: Pos, color: Color) {
            canvas.set_draw_color(color);
            let begin = self.cell_begin(pos);
            if self.config.cell_size == 1 {
                canvas.draw_point(begin).unwrap();
            } else {
                canvas
                    .fill_rect(Rect::new(
                        begin.x,
                        begin.y,
                        self.config.cell_size,
                        self.config.cell_size,
                    ))
                    .unwrap();
            }
        }

        fn draw_pixels(&self, canvas: &mut Canvas<Window>, pos: Vec<Pos>, color: Color) {
            canvas.set_draw_color(color);
            let rects = pos
                .iter()
                .map(|p| {
                    let begin = self.cell_begin(*p);
                    Rect::new(
                        begin.x,
                        begin.y,
                        self.config.cell_size,
                        self.config.cell_size,
                    )
                })
                .collect_vec();
            canvas.fill_rects(&rects).unwrap();
        }

        fn draw_diag_line(
            canvas: &mut Canvas<Window>,
            from: Point,
            to: Point,
            color: Color,
            width: usize,
        ) {
            canvas.set_draw_color(color);
            if from == to {
                // NOTE: We skip the line width in this case.
                canvas.draw_point(from).unwrap();
                return;
            }
            canvas.draw_line(from, to).unwrap();
            for mut w in 1..width as i32 {
                if w % 2 == 1 {
                    w = (w + 1) / 2;
                    canvas
                        .draw_line(
                            Point::new(from.x + w, from.y - w + 1),
                            Point::new(to.x + w - 1, to.y - w),
                        )
                        .unwrap();
                    canvas
                        .draw_line(
                            Point::new(from.x - w, from.y + w - 1),
                            Point::new(to.x - w + 1, to.y + w),
                        )
                        .unwrap();
                    canvas
                        .draw_line(
                            Point::new(from.x + w - 1, from.y - w),
                            Point::new(to.x + w, to.y - w + 1),
                        )
                        .unwrap();
                    canvas
                        .draw_line(
                            Point::new(from.x - w + 1, from.y + w),
                            Point::new(to.x - w, to.y + w - 1),
                        )
                        .unwrap();
                } else {
                    w /= 2;
                    canvas
                        .draw_line(
                            Point::new(from.x + w, from.y - w),
                            Point::new(to.x + w, to.y - w),
                        )
                        .unwrap();
                    canvas
                        .draw_line(
                            Point::new(from.x - w, from.y + w),
                            Point::new(to.x - w, to.y + w),
                        )
                        .unwrap();
                }
            }
        }

        #[allow(unused)]
        fn draw_thick_line_horizontal(
            canvas: &mut Canvas<Window>,
            from: Point,
            to: Point,
            width: i32,
            margin: i32,
        ) {
            for w in -width / 2..width - width / 2 {
                canvas
                    .draw_line(
                        Point::new(from.x + margin, from.y + w),
                        Point::new(to.x - margin, to.y + w),
                    )
                    .unwrap();
            }
        }

        //Saves canvas to bmp file
        fn save_canvas(&self, canvas: &mut Canvas<Window>, last: bool, suffix: Option<&str>) {
            let extension = suffix.map_or("bmp".to_string(), |s| s.to_string() + ".bmp");
            let path = if last {
                let file = path::Path::new(&self.config.filepath);
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                file.with_extension(extension).to_owned()
            } else {
                // Make sure the directory exists.
                let mut dir = path::PathBuf::from(&self.config.filepath);
                std::fs::create_dir_all(&dir).unwrap();
                dir.push(self.file_number.to_string());
                dir.set_extension(extension);
                dir
            };

            let pixel_format = canvas.default_pixel_format();
            let mut pixels = canvas.read_pixels(canvas.viewport(), pixel_format).unwrap();
            let (width, height) = canvas.output_size().unwrap();
            let pitch = pixel_format.byte_size_of_pixels(width as usize);
            let mut surf = sdl2::surface::Surface::from_data(
                pixels.as_mut_slice(),
                width,
                height,
                pitch as u32,
                pixel_format,
            )
            .unwrap();
            if self.config.transparent_bmp {
                surf.set_color_key(true, self.config.style.bg_color)
                    .unwrap();
            }

            surf.save_bmp(path).unwrap_or_else(|error| {
                print!("Problem saving the file: {:?}", error);
            });
        }

        fn draw<'a, H: HeuristicInstance<'a>>(
            &mut self,
            is_last: bool,
            cigar: Option<&Cigar>,
            is_new_layer: bool,
            h: Option<&H>,
            parent: ParentFn,
        ) {
            self.frame_number += 1;
            if is_new_layer {
                self.layer_number += 1;
            }
            if !self.config.draw.is_active(
                self.frame_number,
                self.layer_number,
                is_last,
                is_new_layer,
            ) && !self.config.save.is_active(
                self.frame_number,
                self.layer_number,
                is_last,
                is_new_layer,
            ) && !(is_last && self.config.save_last)
            {
                return;
            }

            // DRAW
            {
                // Draw background.
                let Some(canvas) = &self.canvas else {return;};
                let mut canvas = canvas.borrow_mut();
                canvas.set_draw_color(self.config.style.bg_color);
                canvas
                    .fill_rect(Rect::new(0, 0, self.canvas_size.0, self.canvas_size.1))
                    .unwrap();

                // Draw heuristic values.
                if self.config.style.draw_heuristic && let Some(h) = h {
                    let mut hint = Default::default();
                    let h_max = self.config.style.max_heuristic.unwrap_or(h.h(Pos(0,0)));
                    let mut value_pos_map = HashMap::<u32, Vec<Pos>>::default();
                    for i in 0..=self.target.0 {
                        hint = h.h_with_hint(Pos(i,0), hint).1;
                        let mut hint = hint;
                        for j in 0..=self.target.1 {
                            let pos = Pos(i, j);
                            let (h, new_hint) = h.h_with_hint(pos, hint);
                            hint = new_hint;
                            value_pos_map.entry(h).or_default().push(pos);
                        }
                    }
                    for (h, poss) in value_pos_map {
                        self.draw_pixels(
                            &mut canvas,
                            poss,
                            self.config.style.heuristic.color(h as f64 / h_max as f64),
                        );
                    }
                }

                // Draw layers and contours.
                if self.config.style.draw_contours && let Some(h) = h && h.layer(Pos(0,0)).is_some() {
                    canvas.set_draw_color(self.config.style.contour);
                    let draw_right_border = |canvas: &mut Canvas<Window>, Pos(i, j): Pos| {
                        canvas
                            .draw_line(self.cell_begin(Pos(i + 1, j)), self.cell_begin(Pos(i + 1, j + 1)))
                            .unwrap();
                    };
                    let draw_bottom_border = |canvas: &mut Canvas<Window>, Pos(i, j): Pos| {
                        canvas
                            .draw_line(self.cell_begin(Pos(i, j + 1)), self.cell_begin(Pos(i + 1, j + 1)))
                            .unwrap();
                    };


                    // Right borders
                    let mut hint = Default::default();
                    let mut top_borders = vec![(0, h.layer(Pos(0,0)).unwrap())];
                    for i in 0..self.target.0 {
                        hint = h.layer_with_hint(Pos(i, 0), hint).unwrap().1;
                        let mut hint = hint;
                        for j in 0..=self.target.1 {
                            let pos = Pos(i, j);
                            let (v, new_hint) = h.layer_with_hint(pos, hint).unwrap();
                            hint = new_hint;
                            let pos_r = Pos(i + 1, j);
                            let (v_r, new_hint) = h.layer_with_hint(pos_r, hint).unwrap();
                            hint = new_hint;
                            if v_r != v {
                                draw_right_border(&mut canvas, pos);

                                if j == 0 {
                                    top_borders.push((i+1, v_r));
                                }
                            }
                        }
                    }
                    top_borders.push((self.target.0+1, 0));

                    // Bottom borders
                    let mut hint = Default::default();
                    let mut left_borders = vec![(0, h.layer(Pos(0,0)).unwrap())];
                    for i in 0..=self.target.0 {
                        hint = h.layer_with_hint(Pos(i, 0), hint).unwrap().1;
                        let mut hint = hint;
                        for j in 0..self.target.1 {
                            let pos = Pos(i, j);
                            let (v, new_hint) = h.layer_with_hint(pos, hint).unwrap();
                            hint = new_hint;
                            let pos_l = Pos(i, j + 1);
                            let (v_l, new_hint) = h.layer_with_hint(pos_l, hint).unwrap();
                            hint = new_hint;
                            if v_l != v {
                                draw_bottom_border(&mut canvas, pos);

                                if i == 0 {
                                    left_borders.push((j+1, v_l));
                                }
                            }
                        }
                    }
                    left_borders.push((self.target.1, 0));

                    // Draw numbers at the top and left.
                    for (&(_left, layer), &(right, _)) in top_borders.iter().tuple_windows() {
                        if right < 10 { continue; }
                        let x = (right * self.config.cell_size -1 ).saturating_sub(1);
                        self.write_label(x as i32, -6, HAlign::Right, VAlign::Top, &mut canvas, &layer.to_string());
                    }
                    for (&(_top, layer), &(bottom, _)) in left_borders.iter().tuple_windows(){
                        if bottom < 10 { continue; }
                        let y = bottom * self.config.cell_size +5;
                        self.write_label(3, y as i32, HAlign::Left, VAlign::Bottom, &mut canvas, &layer.to_string());
                    }
                }

                if self.config.draw_old_on_top {
                    // Explored
                    if let Some(color) = self.config.style.explored {
                        for &(t, pos, _, _) in &self.expanded {
                            if t == Type::Explored {
                                self.draw_pixel(&mut canvas, pos, color);
                            }
                        }
                    }
                    // Expanded
                    let mut current_layer = self.layer.unwrap_or(0);
                    for (i, &(t, pos, _, _)) in self.expanded.iter().enumerate().rev() {
                        if t == Type::Explored {
                            continue;
                        }
                        if t == Type::Extended && let Some(c) = self.config.style.extended {
                            self.draw_pixel(&mut canvas, pos, c);
                            continue;
                        }
                        self.draw_pixel(
                            &mut canvas,
                            pos,
                            self.config.style.expanded.color(
                                if let Some(layer) = self.layer && layer != 0 {
                                    if current_layer > 0
                                        && i < self.expanded_layers[current_layer - 1]
                                    {
                                        current_layer -= 1;
                                    }
                                    current_layer as f64 / self.config.num_layers.unwrap_or(layer) as f64
                                } else {
                                        i as f64 / self.expanded.len() as f64
                                },
                            ),
                        );
                    }
                } else {
                    // Explored
                    if let Some(color) = self.config.style.explored {
                        for &(t, pos, _, _) in &self.expanded {
                            if t == Type::Explored {
                                self.draw_pixel(&mut canvas, pos, color);
                            }
                        }
                    }
                    // Expanded
                    let mut current_layer = 0;
                    for (i, &(t, pos, _, _)) in self.expanded.iter().enumerate() {
                        if t == Type::Explored {
                            continue;
                        }
                        if t == Type::Extended && let Some(c) = self.config.style.extended {
                            self.draw_pixel(&mut canvas, pos, c);
                            continue;
                        }
                        self.draw_pixel(
                            &mut canvas,
                            pos,
                            self.config.style.expanded.color(
                                if let Some(layer) = self.layer && layer != 0 {
                                    if current_layer < layer && i >= self.expanded_layers[current_layer] {
                                        current_layer += 1;
                                    }
                                    current_layer as f64 / self.config.num_layers.unwrap_or(layer) as f64
                                } else {
                                        i as f64 / self.expanded.len() as f64
                                },
                            ),
                        );
                    }
                }

                // Draw matches.
                if self.config.style.draw_matches && let  Some(h) = h && let Some(matches) = h.matches() {
                    for m in &matches {
                        if m.match_cost > 0 {
                            continue;
                        }
                        let mut b = self.cell_center(m.start);
                        b.x += self.config.style.match_shrink as i32;
                        b.y += self.config.style.match_shrink as i32;
                        let mut e = self.cell_center(m.end);
                        e.x -= self.config.style.match_shrink as i32;
                        e.y -= self.config.style.match_shrink as i32;
                        Self::draw_diag_line(
                            &mut canvas,
                            b, e,
                            match m.pruned {
                                MatchStatus::Active => self.config.style.active_match,
                                MatchStatus::Pruned => self.config.style.pruned_match,
                            },
                            self.config.style.match_width,
                        );
                    }
                }

                // Draw path.
                if let Some(cigar) = cigar &&
                   let Some(path_color) = self.config.style.path {
                    if let Some(path_width) = self.config.style.path_width {
                        for (from, to) in cigar.to_path().iter().tuple_windows() {
                            Self::draw_diag_line(
                                &mut canvas,
                                self.cell_center(*from),
                                self.cell_center(*to),
                                path_color,
                                path_width,
                            );
                        }
                    } else {
                        for p in cigar.to_path() {
                            self.draw_pixel(&mut canvas, p, path_color)
                        }
                    }
                }

                // Draw tree.
                if let Some(parent) = parent && let Some(tree_color) = self.config.style.tree {
                    for &(_t, u, _, _) in &self.expanded {
                        if self.config.style.tree_fr_only {
                            // Only trace if u is the furthest point on this diagonal.
                            let mut v = u;
                            let mut skip = false;
                            loop {
                                v = v + Pos(1,1);
                                if !(v <= self.target) {
                                    break;
                                }
                                if self.expanded.iter().filter(|(_, u, _, _)| *u == v).count() > 0 {
                                    skip = true;
                                    break;
                                }
                            }
                            if skip {
                                continue;
                            }
                        }
                        let mut st = State{i: u.0 as isize, j: u.1 as isize, layer: None};
                        let mut path = vec![];
                        while let Some((p, op)) = parent(st){
                            path.push((st, p, op));
                            let color = if let Some(CigarOp::AffineOpen(_)) = op[1]
                                && let Some(c) = self.config.style.tree_affine_open {
                                    c
                                } else {
                                    match op[0].unwrap() {
                                        CigarOp::Match => self.config.style.tree_match,
                                        CigarOp::Mismatch => self.config.style.tree_substitution,
                                        _ => None,
                                    }.unwrap_or(tree_color)
                                };
                            Self::draw_diag_line(
                                &mut canvas,
                                self.cell_center(p.pos()),
                                self.cell_center(st.pos()),
                                color,
                                self.config.style.tree_width,
                            );

                            st = p;
                        }
                        if let Some(c) = self.config.style.tree_direction_change {
                            let mut last = CigarOp::Match;
                            for &(u, p, op)  in path.iter().rev() {
                                let op = op[0].unwrap();
                                match op {
                                    CigarOp::Insertion => {
                                        if last == CigarOp::Deletion {
                                            Self::draw_diag_line(
                                                &mut canvas,
                                                self.cell_center(p.pos()),
                                                self.cell_center(u.pos()),
                                                c,
                                                self.config.style.tree_width,
                                            );
                                        }
                                        last = op;
                                    }
                                    CigarOp::Deletion => {
                                        if last == CigarOp::Insertion {
                                            Self::draw_diag_line(
                                                &mut canvas,
                                                self.cell_center(p.pos()),
                                                self.cell_center(u.pos()),
                                                c,
                                                self.config.style.tree_width,
                                            );
                                        }
                                        last = op;
                                    }
                                    CigarOp::Mismatch => {
                                        last = op;
                                    }
                                    _ => {
                                    }
                                }
                            }
                        }
                    }
                } // draw tree

                // Draw labels
                canvas.set_draw_color(Color::BLACK);
                let mut row = 0;
                if let Some(title) = &self.title {
                    self.write_label(
                        self.nw_size.0 as i32 / 2,
                        30 * row,
                        HAlign::Center,
                        VAlign::Top,
                        &mut canvas,
                        title,
                    );
                    row += 1;
                }
                canvas.set_draw_color(Color::RGB(50, 50, 50));
                if let Some(params) = &self.params && !params.is_empty(){
                    self.write_label(
                        self.nw_size.0 as i32 / 2,
                        30 * row,
                        HAlign::Center,
                        VAlign::Top,
                        &mut canvas,
                        params,
                    );
                    row += 1;
                }
                if let Some(comment) = &self.comment && !comment.is_empty(){
                    self.write_label(
                        self.nw_size.0 as i32 / 2,
                        30 * row,
                        HAlign::Center,
                        VAlign::Top,
                        &mut canvas,
                        comment,
                    );
                    row += 1;
                }
                canvas.set_draw_color(Color::GRAY);
                self.write_label(
                    self.nw_size.0 as i32,
                    0,
                    HAlign::Right,
                    VAlign::Top,
                    &mut canvas,
                    &make_label("i = ", self.target.0),
                );
                self.write_label(
                    0,
                    self.nw_size.1 as i32,
                    HAlign::Left,
                    VAlign::Bottom,
                    &mut canvas,
                    &make_label("j = ", self.target.1),
                );

                self.write_label(
                    self.nw_size.0 as i32 / 2,
                    30 * row,
                    HAlign::Center,
                    VAlign::Top,
                    &mut canvas,
                    "DP states (i,j)",
                );
                self.write_label(
                    self.nw_size.0 as i32 / 2,
                    30 * (row + 1),
                    HAlign::Center,
                    VAlign::Top,
                    &mut canvas,
                    &make_label(
                        "expanded: ",
                        self.expanded
                            .iter()
                            .filter(|&(t, ..)| *t == Expanded)
                            .count(),
                    ),
                );
            }

            // Draw DT states
            self.draw_dt(cigar);
            self.draw_f(cigar, h);

            let Some(canvas) = &self.canvas else {return;};
            let mut canvas = canvas.borrow_mut();

            // SAVE

            if self.config.save.is_active(
                self.frame_number,
                self.layer_number,
                is_last,
                is_new_layer,
            ) {
                self.save_canvas(&mut canvas, false, None);
                self.file_number += 1;
            }

            // Save the final frame separately if needed.
            if is_last && self.config.save_last {
                self.save_canvas(&mut canvas, true, None);
            }

            // SHOW

            if !self.config.draw.is_active(
                self.frame_number,
                self.layer_number,
                is_last,
                is_new_layer,
            ) {
                return;
            }

            //Keyboard events

            let sleep_duration = 0.001;
            canvas.present();
            let mut start_time = Instant::now();
            'outer: loop {
                for event in self.sdl_context.event_pump().unwrap().poll_iter() {
                    match event {
                        Event::Quit { .. }
                        | Event::KeyDown {
                            keycode: Some(Keycode::X),
                            ..
                        } => {
                            panic!("Running aborted by user!");
                        }
                        Event::KeyDown {
                            keycode: Some(key), ..
                        } => match key {
                            Keycode::P => {
                                //pause
                                if self.config.paused {
                                    self.config.paused = false;
                                    start_time = Instant::now();
                                } else {
                                    self.config.paused = true;
                                }
                            }
                            Keycode::Escape | Keycode::Space => {
                                //next frame
                                break 'outer;
                            }
                            Keycode::F => {
                                //faster
                                self.config.delay *= 0.8;
                            }
                            Keycode::S => {
                                //slower
                                self.config.delay /= 0.8;
                            }
                            Keycode::Q => {
                                self.config.draw = When::Last;
                                break 'outer;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                ::std::thread::sleep(Duration::from_secs_f32(sleep_duration));

                if !self.config.paused
                    && !is_last
                    && start_time.elapsed().as_secs_f32() >= self.config.delay
                {
                    break 'outer;
                }
            }
        }

        // Draw DT states to the top-right 1/3rd of the canvas.
        fn draw_dt(&mut self, cigar: Option<&Cigar>) {
            if !self.config.show_dt || self.expanded.is_empty() {
                return;
            }
            let Some(canvas) = &self.canvas else {return;};
            let mut canvas = canvas.borrow_mut();

            let offset = (self.nw_size.0 as i32, self.nw_size.1 as i32 / 4);
            // Cell_size goes down in powers of 2.
            let front_max = self.expanded.iter().map(|st| st.2).max().unwrap();
            let diagonal_min = self.expanded.iter().map(|st| st.1.diag()).min().unwrap();
            let diagonal_max = self.expanded.iter().map(|st| st.1.diag()).max().unwrap();
            let dt_cell_size = min(
                self.dt_size.0 / (front_max + 1),
                min(
                    self.dt_size.1 / 2 / max(-diagonal_min + 1, diagonal_max + 1) as u32,
                    10,
                ),
            );

            // Draw grid

            // Divider
            canvas.set_draw_color(Color::BLACK);
            canvas
                .draw_line(
                    Point::new(self.nw_size.0 as i32, 0),
                    Point::new(self.nw_size.0 as i32, self.nw_size.1 as i32),
                )
                .unwrap();

            // Horizontal d lines
            canvas.set_draw_color(Color::GRAY);

            let dy = |d: i32| offset.1 - d * dt_cell_size as i32 - dt_cell_size as i32 / 2;

            let mut draw_d_line = |d: i32, y: i32| {
                canvas
                    .draw_line(
                        Point::new(self.nw_size.0 as i32, y),
                        Point::new(self.canvas_size.0 as i32, y),
                    )
                    .unwrap();
                self.write_label(
                    self.nw_size.0 as i32,
                    y,
                    HAlign::Right,
                    VAlign::Center,
                    &mut canvas,
                    &make_label("d = ", d),
                );
            };
            // d=0
            draw_d_line(0, offset.1);
            // d=min
            if diagonal_min != 0 {
                draw_d_line(diagonal_min, dy(diagonal_min - 1));
            }
            // d=max
            if diagonal_max != 0 {
                draw_d_line(diagonal_max, dy(diagonal_max));
            }

            // Vertical g lines
            canvas.set_draw_color(Color::GRAY);
            let mut draw_g_line = |g: i32| {
                let line_g = if g == 0 { 0 } else { g + 1 };
                let x = self.nw_size.0 as i32 + line_g * dt_cell_size as i32;
                canvas
                    .draw_line(Point::new(x, 0), Point::new(x, self.canvas_size.1 as i32))
                    .unwrap();
                self.write_label(
                    x,
                    dy(diagonal_min - 1),
                    if g == 0 { HAlign::Left } else { HAlign::Right },
                    VAlign::Top,
                    &mut canvas,
                    &make_label("g = ", g),
                );
            };
            // g=0
            draw_g_line(0);
            // g=min
            if front_max > 2 {
                draw_g_line(front_max as i32);
            }

            let state_coords = |st: (Pos, Cost)| -> (i32, i32) {
                (offset.0 + (dt_cell_size * st.1) as i32, dy(st.0.diag()))
            };

            let draw_state =
                |canvas: &mut RefMut<Canvas<Window>>, color: Color, st: (Type, Pos, Cost, Cost)| {
                    canvas.set_draw_color(color);
                    let (x, y) = state_coords((st.1, st.2));
                    canvas
                        .fill_rect(Rect::new(x, y, dt_cell_size, dt_cell_size))
                        .unwrap();
                };

            if self.config.draw_old_on_top {
                // Expanded
                let mut current_layer = self.layer.unwrap_or(0);
                for (i, &st) in self.expanded.iter().enumerate().rev() {
                    let color = self.config.style.expanded.color(
                            if let Some(layer) = self.layer && layer != 0 {
                                if current_layer > 0
                                    && i < self.expanded_layers[current_layer - 1]
                                {
                                    current_layer -= 1;
                                }
                                current_layer as f64 / self.config.num_layers.unwrap_or(layer) as f64
                            } else {
                                    i as f64 / self.expanded.len() as f64
                            },
                        );
                    draw_state(&mut canvas, color, st);
                }
            } else {
                // Expanded
                let mut current_layer = 0;
                for (i, &st) in self.expanded.iter().enumerate() {
                    let color =
                        self.config.style.expanded.color(
                            if let Some(layer) = self.layer && layer != 0 {
                                if current_layer < layer && i >= self.expanded_layers[current_layer] {
                                    current_layer += 1;
                                }
                                current_layer as f64 / self.config.num_layers.unwrap_or(layer) as f64
                            } else {
                                    i as f64 / self.expanded.len() as f64
                            },
                        );
                    draw_state(&mut canvas, color, st);
                }
            }

            // Title
            canvas.set_draw_color(Color::GRAY);
            self.write_label(
                self.nw_size.0 as i32 + self.dt_size.0 as i32 / 2,
                0,
                HAlign::Center,
                VAlign::Top,
                &mut canvas,
                "Diagonal Transition states (g, d) = (s, k)",
            );

            if let Some(cigar) = cigar {
                if let Some(path_color) = self.config.style.path {
                    for (from, to) in cigar
                        .to_path_with_cost(LinearCost::new_unit())
                        .iter()
                        .tuple_windows()
                    {
                        let from_coords = state_coords(*from);
                        let to_coords = state_coords(*to);
                        if from_coords == to_coords {
                            continue;
                        }
                        if let Some(path_width) = self.config.style.path_width {
                            Self::draw_diag_line(
                                &mut canvas,
                                Point::new(
                                    from_coords.0 + dt_cell_size as i32 / 2,
                                    from_coords.1 + dt_cell_size as i32 / 2,
                                ),
                                Point::new(
                                    to_coords.0 + dt_cell_size as i32 / 2,
                                    to_coords.1 + dt_cell_size as i32 / 2,
                                ),
                                path_color,
                                path_width,
                            );
                        } else {
                            draw_state(&mut canvas, path_color, (Expanded, from.0, from.1, 0));
                        }
                    }
                }
            }
        }

        fn write_label(
            &self,
            x: i32,
            y: i32,
            ha: HAlign,
            va: VAlign,
            canvas: &mut std::cell::RefMut<Canvas<Window>>,
            text: &str,
        ) {
            // Add labels
            #[cfg(feature = "sdl2_ttf")]
            {
                let surface = self.font.render(text).blended(canvas.draw_color()).unwrap();
                let w = surface.width();
                let h = surface.height();
                let x = match ha {
                    HAlign::Left => x,
                    HAlign::Center => x - w as i32 / 2,
                    HAlign::Right => x - w as i32,
                };
                let y = match va {
                    VAlign::Top => y,
                    VAlign::Center => y - h as i32 / 2,
                    VAlign::Bottom => y - h as i32,
                };
                let texture_creator = canvas.texture_creator();
                canvas
                    .copy(
                        &surface.as_texture(&texture_creator).unwrap(),
                        None,
                        Some(Rect::new(x, y, w, h)),
                    )
                    .unwrap();
            }
        }

        fn draw_f<'a, H: HeuristicInstance<'a>>(&mut self, cigar: Option<&Cigar>, h: Option<&H>) {
            if !self.config.show_fronts || self.expanded.is_empty() {
                return;
            }
            let Some(canvas) = &self.canvas else {return;};
            let mut canvas = canvas.borrow_mut();

            // Soft red
            const SOFT_RED: Color = Color::RGB(244, 113, 116);
            const _SOFT_GREEN: Color = Color::RGB(111, 194, 118);

            // Cell size from DT
            // Cell_size goes down in powers of 2.
            let front_max = self.expanded.iter().map(|st| st.2).max().unwrap();
            let diagonal_min = self.expanded.iter().map(|st| st.1.diag()).min().unwrap();
            let diagonal_max = self.expanded.iter().map(|st| st.1.diag()).max().unwrap();
            let dt_cell_size = min(
                self.dt_size.0 / (front_max + 1),
                min(
                    self.dt_size.1 / 2 / max(-diagonal_min + 1, diagonal_max + 1) as u32,
                    10,
                ),
            );

            // f is plotted with f_min at y=height-30, and f_max at y=3/4*height
            let f_min = self
                .expanded
                .iter()
                .filter(|st| st.0 == Expanded)
                .map(|st| st.3)
                .min()
                .unwrap();
            let f_max = self
                .expanded
                .iter()
                .filter(|st| st.0 == Expanded)
                .map(|st| st.3)
                .max()
                .unwrap();
            let f_y = |f| {
                self.canvas_size.1 as i32
                    - ((f - f_min) as f32 / (f_max - f_min) as f32 * self.canvas_size.1 as f32 / 4.)
                        as i32
                    - 30
            };

            // Draw shifted states after pruning.
            if let Some(h) = h {
                for &(t, pos, g, _) in self.expanded.iter() {
                    if t == Explored {
                        continue;
                    }
                    let f = g + h.h(pos);
                    let rel_f = (f - f_min) as f64 / max(f_max - f_min, 1) as f64;
                    if rel_f > 1.5 {
                        continue;
                    }
                    canvas.set_draw_color(
                        Gradient::Gradient(Color::GRAY..Color::WHITE)
                            .color(f64::max(0., 2. * rel_f - 2.)),
                    );
                    let y = f_y(f);
                    canvas
                        .fill_rect(Rect::new(
                            (pos.0 * self.config.cell_size) as i32,
                            y,
                            self.config.cell_size,
                            1,
                        ))
                        .unwrap();
                    canvas
                        .fill_rect(Rect::new(
                            self.nw_size.0 as i32 + (g * dt_cell_size) as i32,
                            y,
                            dt_cell_size,
                            1,
                        ))
                        .unwrap();
                }
            }

            for (i, &(t, pos, g, f)) in self.expanded.iter().enumerate() {
                if t == Explored {
                    continue;
                }
                canvas.set_draw_color(
                    //Gradient::Gradient(SOFT_GREEN..SOFT_RED)
                    Gradient::TurboGradient(0.2..0.95).color(i as f64 / self.expanded.len() as f64),
                );
                canvas
                    .fill_rect(Rect::new(
                        (pos.0 * self.config.cell_size) as i32,
                        f_y(f),
                        self.config.cell_size,
                        2,
                    ))
                    .unwrap();
                canvas
                    .fill_rect(Rect::new(
                        self.nw_size.0 as i32 + (g * dt_cell_size) as i32,
                        f_y(f),
                        dt_cell_size,
                        2,
                    ))
                    .unwrap();
            }

            // Horizontal line at final cost when path is given.
            let mut cost = None;
            if let Some(cigar) = cigar {
                let c = cigar
                    .to_path_with_cost(LinearCost::new_unit())
                    .last()
                    .unwrap()
                    .1;
                cost = Some(c);
                let y = f_y(c);
                canvas.set_draw_color(SOFT_RED);
                canvas
                    .draw_line(Point::new(0, y), Point::new(self.canvas_size.0 as i32, y))
                    .unwrap();

                canvas.set_draw_color(SOFT_RED);
                self.write_label(
                    self.nw_size.0 as i32,
                    y,
                    HAlign::Left,
                    VAlign::Center,
                    &mut canvas,
                    &make_label("g* = ", c),
                );
            };

            canvas.set_draw_color(SOFT_RED);
            self.write_label(
                self.nw_size.0 as i32 + self.dt_size.0 as i32 / 2,
                self.dt_size.1 as i32,
                HAlign::Center,
                VAlign::Bottom,
                &mut canvas,
                "max f per front g",
            );
            for f in [f_min, f_max] {
                if Some(f) == cost {
                    continue;
                }
                self.write_label(
                    self.nw_size.0 as i32,
                    f_y(f),
                    HAlign::Left,
                    VAlign::Center,
                    &mut canvas,
                    &make_label("f = ", f),
                );
            }

            self.write_label(
                self.nw_size.0 as i32 / 2,
                self.nw_size.1 as i32,
                HAlign::Center,
                VAlign::Bottom,
                &mut canvas,
                "max f per column i",
            );
        }
    }
}
