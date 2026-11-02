//! Frame-time measurement for performance work, off unless `SE_PERF=FILE` is set. The `perf NAME`
//! dev command starts a named segment (ending the one before), `perf end` ends it; each finished
//! segment appends one line to FILE: frame count, mean / median / 99th-percentile / worst frame
//! time, the 1% low frame rate, and the GPU and CPU time of each timed render pass per frame
//! (Bevy's render diagnostics, Vulkan timestamp queries), and the mean entity count.

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore};
use bevy::platform::collections::HashMap;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use std::io::Write;

pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("SE_PERF") else { return };
        app.add_plugins((bevy::render::diagnostic::RenderDiagnosticsPlugin, bevy::diagnostic::EntityCountDiagnosticsPlugin::default()))
            .insert_resource(PerfLog { path: path.into(), ..default() })
            .add_systems(Last, record_frame);
    }
}

#[derive(Resource, Default)]
pub struct PerfLog {
    path: std::path::PathBuf,
    name: Option<String>,
    frames_ms: Vec<f32>,
    /// Summed entity count (one sample a frame).
    entities: f64,
    /// Render pass diagnostics: summed value (ms) over the segment.
    passes: HashMap<String, f64>,
    /// The newest measurement already counted, per diagnostic.
    seen: HashMap<DiagnosticPath, Instant>,
}

impl PerfLog {
    /// `perf NAME` / `perf end`.
    pub fn mark(&mut self, name: &str) {
        self.finish();
        if name != "end" && !name.is_empty() {
            self.name = Some(name.to_string());
        }
    }

    fn finish(&mut self) {
        let Some(name) = self.name.take() else { return };
        let mut ms = std::mem::take(&mut self.frames_ms);
        let passes = std::mem::take(&mut self.passes);
        let entities = std::mem::take(&mut self.entities);
        if ms.is_empty() {
            return;
        }
        let n = ms.len();
        let mean = ms.iter().map(|&v| v as f64).sum::<f64>() / n as f64;
        ms.sort_by(f32::total_cmp);
        let pct = |p: f64| ms[((n as f64 - 1.0) * p).round() as usize];
        let worst = (n / 100).max(1);
        let low = ms[n - worst..].iter().map(|&v| v as f64).sum::<f64>() / worst as f64;
        let mut line = format!(
            "{name}\tframes {n}\tmean {mean:.2} ms ({:.0} fps)\tmedian {:.2}\tp99 {:.2}\tmax {:.2}\t1% low {:.0} fps",
            1000.0 / mean,
            pct(0.5),
            pct(0.99),
            ms[n - 1],
            1000.0 / low
        );
        line += &format!("\tentities {:.0}", entities / n as f64);
        let mut passes: Vec<_> = passes.into_iter().collect();
        passes.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (p, sum) in passes.iter().filter(|(_, s)| *s / n as f64 >= 0.05) {
            line += &format!("\t{p} {:.2}", sum / n as f64);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{line}");
        }
        info!("perf: {line}");
    }
}

fn record_frame(mut log: ResMut<PerfLog>, time: Res<Time<Real>>, store: Res<DiagnosticsStore>) {
    let log = &mut *log;
    // Diagnostics arrive a few frames late; take every measurement newer than the last one counted.
    for d in store.iter() {
        let path = d.path();
        let p = path.as_str();
        if !p.starts_with("render/") || !(p.ends_with("elapsed_gpu") || p.ends_with("elapsed_cpu")) {
            continue;
        }
        let last = log.seen.get(path).copied();
        let mut newest = last;
        for m in d.measurements() {
            if last.is_some_and(|t| m.time <= t) {
                continue;
            }
            newest = Some(newest.map_or(m.time, |t: Instant| t.max(m.time)));
            if log.name.is_some() {
                *log.passes.entry(p.trim_start_matches("render/").to_string()).or_default() += m.value;
            }
        }
        if let Some(t) = newest {
            log.seen.insert(path.clone(), t);
        }
    }
    if log.name.is_some() {
        log.frames_ms.push(time.delta_secs() * 1000.0);
        log.entities += store.get(&bevy::diagnostic::EntityCountDiagnosticsPlugin::ENTITY_COUNT).and_then(|d| d.value()).unwrap_or(0.0);
    }
}
