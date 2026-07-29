//! Frame timing for the render path, enabled with `OPTION_TERM_PROFILE=1`.
//!
//! `Session::paint` is split into phases so a baseline can be recorded before
//! the renderer is reworked and compared against afterwards. When the env var
//! is absent every method here is a branch on a `bool`, so the release build
//! keeps paying nothing.

use std::time::{Duration, Instant};

/// Frames aggregated before a summary is emitted.
const WINDOW: usize = 120;

/// Summaries are also emitted on this cadence. A terminal only repaints when
/// something changes, so a frame-only trigger can stay silent for a whole run.
const REPORT_EVERY: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy)]
pub enum Phase {
    /// Background fill of the whole surface.
    Clear,
    /// Font descriptions, render snapshot, palette, cursor state.
    Setup,
    /// Kitty placements below the text layer.
    ImagesBelow,
    /// The row/cell loop: backgrounds, glyphs, decorations.
    Cells,
    /// Nested inside `Cells`: only the Pango layout + `show_layout` per cell.
    /// Timing this costs two `Instant::now()` per glyph, so it inflates `cells`
    /// by a percent or so; it is here to size the win from batching/caching.
    Glyphs,
    /// Non-block cursor shapes drawn on top.
    Cursor,
    /// Kitty placements above the text layer.
    ImagesAbove,
}

impl Phase {
    const ALL: [Phase; 7] = [
        Self::Clear,
        Self::Setup,
        Self::ImagesBelow,
        Self::Cells,
        Self::Glyphs,
        Self::Cursor,
        Self::ImagesAbove,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Setup => "setup",
            Self::ImagesBelow => "img_below",
            Self::Cells => "cells",
            Self::Glyphs => "  └ pango",
            Self::Cursor => "cursor",
            Self::ImagesAbove => "img_above",
        }
    }
}

const PHASES: usize = Phase::ALL.len();

#[derive(Clone, Copy, Default)]
struct Sample {
    phases: [Duration; PHASES],
    total: Duration,
    /// Wall clock since the previous frame, i.e. the effective repaint period.
    interval: Duration,
    cells: u32,
    /// Pango layout + `show_layout` round trips, the cost we suspect dominates.
    glyphs: u32,
}

/// Accumulates one frame's timings. Inert unless profiling is on.
pub struct Frame {
    start: Option<Instant>,
    last: Option<Instant>,
    sample: Sample,
}

impl Frame {
    fn disabled() -> Self {
        Self {
            start: None,
            last: None,
            sample: Sample::default(),
        }
    }

    /// Charges the time since the previous mark to `phase`.
    pub fn mark(&mut self, phase: Phase) {
        let Some(last) = self.last else { return };
        let now = Instant::now();
        self.sample.phases[phase as usize] += now - last;
        self.last = Some(now);
    }

    /// Opens a nested measurement; pair with [`Frame::add`].
    pub fn nested(&mut self) -> Option<Instant> {
        self.start.map(|_| Instant::now())
    }

    /// Charges a nested block to `phase` without disturbing the mark cursor.
    pub fn add(&mut self, phase: Phase, since: Option<Instant>) {
        if let Some(at) = since {
            self.sample.phases[phase as usize] += at.elapsed();
        }
    }

    pub fn count_cell(&mut self) {
        if self.start.is_some() {
            self.sample.cells += 1;
        }
    }

    pub fn count_glyph(&mut self) {
        if self.start.is_some() {
            self.sample.glyphs += 1;
        }
    }
}

pub struct Profiler {
    enabled: bool,
    samples: Vec<Sample>,
    last_frame: Option<Instant>,
    /// Start of the window currently being accumulated.
    window_start: Option<Instant>,
    /// Last grid seen, so `Drop` can label the trailing partial window.
    grid: (u16, u16),
}

impl Profiler {
    pub fn new() -> Self {
        let enabled = std::env::var_os("OPTION_TERM_PROFILE").is_some_and(|v| v != "0");
        if enabled {
            tracing::info!(
                "render profiling on; summary every {WINDOW} frames or {}ms",
                REPORT_EVERY.as_millis()
            );
        }
        Self {
            enabled,
            samples: Vec::with_capacity(WINDOW),
            last_frame: None,
            window_start: None,
            grid: (0, 0),
        }
    }

    pub fn begin(&mut self) -> Frame {
        if !self.enabled {
            return Frame::disabled();
        }
        let now = Instant::now();
        let interval = self.last_frame.map(|prev| now - prev).unwrap_or_default();
        self.last_frame = Some(now);
        Frame {
            start: Some(now),
            last: Some(now),
            sample: Sample {
                interval,
                ..Sample::default()
            },
        }
    }

    pub fn end(&mut self, frame: Frame, cols: u16, rows: u16) {
        let Some(start) = frame.start else { return };
        let mut sample = frame.sample;
        sample.total = start.elapsed();
        self.samples.push(sample);
        self.grid = (cols, rows);
        let opened = *self.window_start.get_or_insert(start);
        if self.samples.len() >= WINDOW || opened.elapsed() >= REPORT_EVERY {
            self.report(cols, rows);
            self.samples.clear();
            self.window_start = None;
        }
    }

    fn report(&self, cols: u16, rows: u16) {
        let n = self.samples.len();
        if n == 0 {
            return;
        }
        let total = Stats::of(self.samples.iter().map(|s| s.total));
        // The first frame of a window has no predecessor, so it reports a zero
        // interval that would drag the median down.
        let interval = Stats::of(
            self.samples
                .iter()
                .map(|s| s.interval)
                .filter(|d| !d.is_zero()),
        );
        let cells = self.samples.iter().map(|s| s.cells as u64).sum::<u64>() / n as u64;
        let glyphs = self.samples.iter().map(|s| s.glyphs as u64).sum::<u64>() / n as u64;

        tracing::info!(
            "frames={n} grid={cols}x{rows} cells/frame={cells} glyphs/frame={glyphs} \
             paint p50={:.2}ms p99={:.2}ms max={:.2}ms | period p50={:.2}ms (~{:.0} fps)",
            total.p50,
            total.p99,
            total.max,
            interval.p50,
            if interval.p50 > 0.0 {
                1000.0 / interval.p50
            } else {
                0.0
            },
        );
        for phase in Phase::ALL {
            let s = Stats::of(self.samples.iter().map(|x| x.phases[phase as usize]));
            let share = if total.sum > 0.0 {
                100.0 * s.sum / total.sum
            } else {
                0.0
            };
            tracing::info!(
                "  {:<9} p50={:>7.3}ms p99={:>7.3}ms  {share:>4.1}% of paint",
                phase.label(),
                s.p50,
                s.p99,
            );
        }
    }
}

impl Drop for Profiler {
    /// A run that ends mid-window (the workload finished, the pane closed)
    /// would otherwise throw away every frame it collected.
    fn drop(&mut self) {
        if !self.samples.is_empty() {
            tracing::info!("final partial window:");
            self.report(self.grid.0, self.grid.1);
        }
    }
}

/// Percentiles in milliseconds.
struct Stats {
    p50: f64,
    p99: f64,
    max: f64,
    sum: f64,
}

impl Stats {
    fn of(values: impl Iterator<Item = Duration>) -> Self {
        let mut ms: Vec<f64> = values.map(|d| d.as_secs_f64() * 1000.0).collect();
        if ms.is_empty() {
            return Self {
                p50: 0.0,
                p99: 0.0,
                max: 0.0,
                sum: 0.0,
            };
        }
        let sum = ms.iter().sum();
        ms.sort_unstable_by(f64::total_cmp);
        let at = |q: f64| ms[(((ms.len() - 1) as f64) * q).round() as usize];
        Self {
            p50: at(0.5),
            p99: at(0.99),
            max: ms[ms.len() - 1],
            sum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_track_the_sorted_input() {
        // Nearest-rank over indices 0..=99: p50 lands on index 50, not 49.
        let s = Stats::of((1..=100).map(Duration::from_millis));
        assert_eq!(s.p50, 51.0);
        assert_eq!(s.p99, 99.0);
        assert_eq!(s.max, 100.0);
        assert_eq!(s.sum, (1..=100).sum::<u64>() as f64);
    }

    #[test]
    fn an_empty_window_is_all_zeroes() {
        let s = Stats::of(std::iter::empty());
        assert_eq!((s.p50, s.p99, s.max, s.sum), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn a_disabled_profiler_records_nothing() {
        let mut p = Profiler {
            enabled: false,
            samples: Vec::new(),
            last_frame: None,
            window_start: None,
            grid: (0, 0),
        };
        let mut frame = p.begin();
        frame.mark(Phase::Cells);
        frame.count_glyph();
        p.end(frame, 80, 24);
        assert!(p.samples.is_empty());
    }
}
