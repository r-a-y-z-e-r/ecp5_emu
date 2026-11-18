//! The window's load clock and a card over the interactive, empty die.
use crate::{start::StageTimes, theme};
use eframe::egui::{self, FontId, Rect, RichText, Vec2};
use std::{path::PathBuf, time::Instant};

const CHOOSING: &str = "Choosing fast mode's settings";

/// An overlay's surface uses theme colours and never allocates panel space.
pub fn card_surface(ui: &mut egui::Ui, id: &str, rect: Rect) -> egui::Ui {
    let mut card = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    card.set_clip_rect(rect.intersect(ui.clip_rect()));
    card.interact(rect, card.id().with("surface"), egui::Sense::hover());
    card.painter().rect(
        rect,
        10.0,
        card.visuals().window_fill(),
        card.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    card
}

/// Trials belong to one stage: a new trial changes its label, not its clock.
fn stage(text: &str) -> (&str, Option<(usize, usize)>) {
    if let Some(trial) = text
        .strip_prefix(CHOOSING)
        .and_then(|s| s.strip_prefix(": "))
    {
        let mut words = trial.split_whitespace();
        let counts = (|| {
            let n = words.next()?.parse().ok()?;
            (words.next()? == "of" && words.next()? == "about").then_some(())?;
            let total = words.next()?.parse().ok()?;
            Some((n, total))
        })();
        (CHOOSING, counts)
    } else {
        (text, None)
    }
}

#[derive(Debug, PartialEq)]
struct Progress {
    fraction: f32,
    remaining: f64,
}

/// The previous load supplies the weights. A slow stage spends its own
/// budget, never the stages still to come; only readiness fills the bar.
fn progress(
    remembered: &[(String, f64)],
    current: &str,
    seconds: f64,
    ready: bool,
) -> Option<Progress> {
    let total: f64 = remembered.iter().map(|(_, seconds)| seconds).sum();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    if ready {
        return Some(Progress {
            fraction: 1.0,
            remaining: 0.0,
        });
    }
    let at = remembered
        .iter()
        .position(|(name, _)| name == stage(current).0)?;
    // Reaching this stage completes the earlier weights, even if a short
    // stage began and ended between the window's frames.
    let before: f64 = remembered[..at].iter().map(|(_, seconds)| seconds).sum();
    let budget = remembered[at].1;
    let after: f64 = remembered[at + 1..]
        .iter()
        .map(|(_, seconds)| seconds)
        .sum();
    Some(Progress {
        fraction: ((before + seconds.clamp(0.0, budget)) / total).clamp(0.0, 0.99) as f32,
        remaining: (budget - seconds).max(0.0) + after,
    })
}

pub struct Loading {
    pub path: PathBuf,
    opened: Instant,
    changed: Instant,
    current: String,
    done: StageTimes,
    remembered: StageTimes,
    pub die_shown: bool,
}

impl Loading {
    pub fn new(path: PathBuf, remembered: StageTimes, now: Instant) -> Self {
        Self {
            path,
            opened: now,
            changed: now,
            current: String::new(),
            done: Vec::new(),
            remembered,
            die_shown: false,
        }
    }

    pub fn elapsed(&self, now: Instant) -> f64 {
        now.duration_since(self.opened).as_secs_f64()
    }

    /// Returns a record note for each change seen in Shared::stage.
    pub fn observe(&mut self, text: &str, now: Instant) -> Option<String> {
        if text.is_empty() || text == self.current {
            return None;
        }
        if stage(text).0 != stage(&self.current).0 && !self.current.is_empty() {
            self.complete(now);
        }
        self.current = text.to_owned();
        Some(format!("load {:.3} {text}", self.elapsed(now)))
    }

    fn complete(&mut self, now: Instant) {
        if !self.current.is_empty() && self.current != "Ready" {
            self.done.push((
                stage(&self.current).0.to_owned(),
                now.duration_since(self.changed).as_secs_f64(),
            ));
        }
        self.changed = now;
    }

    pub fn finish(&mut self, now: Instant) -> StageTimes {
        self.complete(now);
        std::mem::take(&mut self.done)
    }

    /// Draws without allocating map space. Text is clipped to fixed rows,
    /// so a long stage or design name cannot move the die or grow the card.
    pub fn card(&self, ui: &mut egui::Ui, map: Rect, title: &str, now: Instant) -> Rect {
        let size = Vec2::new(
            400.0_f32.min(map.width() * 0.6),
            280.0_f32.min(map.height()),
        );
        let rect = Rect::from_center_size(map.center(), size);
        let mut card = card_surface(ui, "loading_card", rect);
        let colours = theme::colours(card.visuals().dark_mode);
        let body = card.visuals().text_color();
        let inner = rect.shrink(14.0);
        let row = |y: f32, height: f32| {
            Rect::from_min_size(
                inner.min + egui::vec2(0.0, y),
                egui::vec2(inner.width(), height),
            )
        };
        let label = |ui: &mut egui::Ui, rect: Rect, text: &str, size, colour| {
            ui.put(
                rect,
                egui::Label::new(
                    RichText::new(text)
                        .font(FontId::proportional(size))
                        .color(colour),
                )
                .truncate(),
            );
        };
        label(&mut card, row(0.0, 24.0), title, 18.0, colours.net);
        let current = if self.current.is_empty() {
            "Opening the design"
        } else {
            &self.current
        };
        let (name, trial) = stage(current);
        label(&mut card, row(32.0, 20.0), name, 13.5, body);
        if let Some((n, total)) = trial {
            label(
                &mut card,
                row(53.0, 18.0),
                &format!("{n} of about {total}"),
                12.5,
                colours.muted,
            );
        }
        let seconds = now.duration_since(self.changed).as_secs_f64();
        label(
            &mut card,
            row(77.0, 18.0),
            &format!(
                "This stage {seconds:.1} s · Total {:.1} s",
                self.elapsed(now)
            ),
            12.5,
            colours.muted,
        );
        for (i, (name, _)) in self.done.iter().rev().take(6).rev().enumerate() {
            label(
                &mut card,
                row(106.0 + i as f32 * 17.0, 17.0),
                &format!("✔ {name}"),
                11.0,
                colours.muted,
            );
        }
        if let Some(estimate) = progress(&self.remembered, current, seconds, false) {
            let bar = row(219.0, 6.0);
            card.painter()
                .rect_filled(bar, 3.0, card.visuals().extreme_bg_color);
            card.painter().rect_filled(
                Rect::from_min_size(
                    bar.min,
                    egui::vec2(bar.width() * estimate.fraction, bar.height()),
                ),
                3.0,
                colours.net,
            );
            label(
                &mut card,
                row(232.0, 18.0),
                &format!("about {:.0} s left", estimate.remaining.ceil()),
                12.5,
                colours.muted,
            );
        } else {
            label(
                &mut card,
                row(232.0, 18.0),
                &format!("{} stages done", self.done.len()),
                12.5,
                colours.muted,
            );
        }
        rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn times() -> StageTimes {
        vec![
            ("Reading the configuration".into(), 6.0),
            ("Compiling the execution plan".into(), 22.0),
            (CHOOSING.into(), 8.0),
        ]
    }

    #[test]
    fn first_load_has_stages_but_no_estimate() {
        let now = Instant::now();
        let mut load = Loading::new("first.config".into(), Vec::new(), now);
        assert!(load.observe("Reading the configuration", now).is_some());
        assert!(load.observe("Reading the configuration", now).is_none());
        let later = now + Duration::from_secs(6);
        assert_eq!(
            load.observe("Compiling the execution plan", later).unwrap(),
            "load 6.000 Compiling the execution plan"
        );
        assert_eq!(load.done, times()[..1]);
        assert_eq!(load.current, "Compiling the execution plan");
        assert_eq!(progress(&[], &load.current, 3.0, false), None);
    }

    #[test]
    fn repeat_load_uses_previous_stage_weights_and_remaining_time() {
        let times = times();
        let estimate = progress(&times, &times[1].0, 4.0, false).unwrap();
        assert!((estimate.fraction - 10.0 / 36.0).abs() < 1e-6);
        assert_eq!(estimate.remaining, 26.0);
        assert_eq!(
            progress(&times, &times[0].0, 0.0, false).unwrap().remaining,
            36.0
        );
        assert_eq!(progress(&times, "An unremembered stage", 1.0, false), None);
        let now = Instant::now();
        let mut repeat = Loading::new("repeat.config".into(), times.clone(), now);
        repeat.observe(&times[1].0, now);
        assert!(progress(&repeat.remembered, &repeat.current, 1.0, false).is_some());
    }

    #[test]
    fn fast_mode_trials_share_one_clock_and_one_budget() {
        let now = Instant::now();
        let mut load = Loading::new("trial.config".into(), Vec::new(), now);
        for (seconds, n) in [(0, 1), (2, 2), (4, 3)] {
            let text = format!("{CHOOSING}: {n} of about 3 (threads 4, folding 8)");
            let at = now + Duration::from_secs(seconds);
            assert!(load.observe(&text, at).is_some());
            assert_eq!(load.changed, now);
            assert!(load.done.is_empty());
            assert_eq!(stage(&text), (CHOOSING, Some((n, 3))));
            let times = vec![(CHOOSING.into(), 8.0)];
            let estimate = progress(&times, &text, seconds as f64, false).unwrap();
            assert_eq!(estimate.remaining, 8.0 - seconds as f64);
            assert_eq!(estimate.fraction, seconds as f32 / 8.0);
        }
        assert_eq!(
            load.finish(now + Duration::from_secs(8)),
            vec![(CHOOSING.into(), 8.0)]
        );
    }

    #[test]
    fn a_slow_stage_keeps_future_time_and_never_finishes_the_bar() {
        let times = times();
        let slow = progress(&times, &times[1].0, 100.0, false).unwrap();
        assert_eq!(slow.remaining, 8.0);
        assert!(slow.fraction <= 0.99);
        let last = progress(&times, CHOOSING, 100.0, false).unwrap();
        assert_eq!(last.remaining, 0.0);
        assert_eq!(last.fraction, 0.99);
        assert_eq!(
            progress(&times, "Ready", 100.0, true).unwrap(),
            Progress {
                fraction: 1.0,
                remaining: 0.0
            }
        );
    }

    #[test]
    fn loading_card_keeps_its_size_and_map_in_both_themes() {
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            let ctx = egui::Context::default();
            theme::install(&ctx, theme);
            for width in [600.0, 1100.0] {
                let mut previous = None;
                for (text, remembered) in [
                    ("Reading the configuration".to_owned(), Vec::new()),
                    (format!("{CHOOSING}: 1 of about 3 (four threads)"), times()),
                    (
                        format!("{CHOOSING}: 300 of about 999 (a much longer description)"),
                        times(),
                    ),
                    (
                        "A very long stage with spaces and\nline breaks ".repeat(30),
                        Vec::new(),
                    ),
                ] {
                    let now = Instant::now();
                    let mut load = Loading::new("test.config".into(), remembered, now);
                    for (name, _) in &times()[..2] {
                        load.observe(name, now);
                    }
                    load.observe(&text, now);
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 700.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                let map = ui.available_rect_before_wrap();
                                ui.allocate_exact_size(map.size(), egui::Sense::click_and_drag());
                                let before = ui.min_rect();
                                let card = load.card(ui, map, &"A design name ".repeat(30), now);
                                assert_eq!(
                                    ui.min_rect(),
                                    before,
                                    "the card must not resize the map"
                                );
                                assert_eq!(card.center(), map.center());
                                assert!(card.width() <= map.width() * 0.6 + 0.01);
                                assert!((card.height() - 280.0).abs() < 0.01);
                                if let Some(previous) = previous {
                                    assert_eq!(card, previous);
                                }
                                previous = Some(card);
                            });
                        },
                    );
                    let texts: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| {
                            if let egui::Shape::Text(text) = &shape.shape {
                                Some(text.galley.text())
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert!(texts.iter().any(|text| text.contains("This stage")));
                    assert!(texts.iter().any(|text| text.contains(
                        if load.remembered.is_empty() {
                            "stages done"
                        } else {
                            "s left"
                        }
                    )));
                }
            }
        }
    }
}
