//! Frame-time measurement for performance work, off unless `SE_PERF=FILE` is set. The `perf NAME`
//! dev command starts a named segment (ending the one before), `perf end` ends it; each finished
//! segment appends one line to FILE: frame count, mean / median / 99th-percentile / worst frame
//! time, the 1% low frame rate, and the GPU and CPU time of each timed render pass per frame
//! (Bevy's render diagnostics, Vulkan timestamp queries), the mean entity count, and at its end the
//! counts of the main assets (images with their bytes, meshes, the ground's and clouds' materials,
//! storage buffers), the pipelines in Bevy's cache and the port's memory (what piles up over a session).

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore};
use bevy::platform::collections::HashMap;
use bevy::platform::time::Instant;
use bevy::ecs::entity_disabling::Disabled;
use bevy::ecs::query::Allow;
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
    /// The asset counts and memory at the latest sample (`record_frame`), written with the segment.
    counts: String,
    /// Frames since the counts were last taken.
    since_counts: u32,
}

impl PerfLog {
    /// `perf NAME` / `perf end`.
    pub fn mark(&mut self, name: &str) {
        // (A marker in a trace build's timeline, `--features bevy/trace_chrome`.)
        info_span!("perf_mark", segment = name).in_scope(|| {});
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
        line += &format!("\tentities {:.0}\t{}", entities / n as f64, self.counts);
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

#[allow(clippy::too_many_arguments)]
fn record_frame(
    mut log: ResMut<PerfLog>,
    time: Res<Time<Real>>,
    store: Res<DiagnosticsStore>,
    images: Res<Assets<Image>>,
    meshes: Res<Assets<Mesh>>,
    (ground_mats, cloud_mats, buffers, pipes): (
        Res<Assets<crate::terrain::PlanetMaterial>>,
        Res<Assets<crate::terrain::CloudMaterial>>,
        Res<Assets<bevy::render::storage::ShaderBuffer>>,
        Res<crate::pipeline_watch::GroundPipelines>,
    ),
    (caches, terrains): (Res<crate::terrain::TileCaches>, Query<&crate::terrain::Terrain>),
    kinds: (
        Query<(), (With<crate::system::SystemMember>, Allow<Disabled>)>,
        Query<(), (With<crate::system::SystemBody>, Allow<Disabled>)>,
        Query<(), (With<crate::terrain::TerrainNode>, Allow<Disabled>)>,
        Query<(), (With<crate::terrain::CloudNode>, Allow<Disabled>)>,
        Query<(), Allow<Disabled>>,
    ),
) {
    let log = &mut *log;
    // The counts once a second or so (the images' bytes take a walk over all of them).
    log.since_counts += 1;
    if log.since_counts >= 60 || log.counts.is_empty() {
        log.since_counts = 0;
        let image_mb = images.iter().map(|(_, i)| i.data.as_ref().map_or(0, |d| d.len())).sum::<usize>() as f64 / 1048576.0;
        let (ws, private) = crate::memory::process_mb().unwrap_or((0, 0));
        let (gpu_n, gpu_mb) = pipes.gpu_images();
        let [w0, w1, w2, w3] = pipes.wgpu_objects();
        let (re, mi) = pipes.render_counts();
        let (tiles, tile_mb, heights) = caches.sizes();
        let all = kinds.4.iter().count();
        let (members, sbodies, tnodes, cnodes) = (kinds.0.iter().count(), kinds.1.iter().count(), kinds.2.iter().count(), kinds.3.iter().count());
        let (bodies, nodes, grids) = terrains.iter().fold((0, 0, 0), |(b, n, g), t| (b + 1, n + t.resident().0, g + t.resident().1));
        log.counts = format!(
            "images {} ({image_mb:.0} MB)\tmeshes {}\tground_mats {}\tcloud_mats {}\tbuffers {}\tpipelines {}\tgpu_images {gpu_n} ({gpu_mb} MB)\ttiles {tiles} ({tile_mb} MB, {heights} heights)\tterrains {bodies} ({nodes} nodes, {grids} grids)\tall_entities {all} (members {members}, bodies {sbodies}, ground nodes {tnodes}, cloud nodes {cnodes})\twgpu {w0} bind groups, {w1} buffers, {w2} textures, {w3} views\trender {re} entities, {mi} mesh instances\tram {ws} MB (private {private} MB)",
            images.len(),
            meshes.len(),
            ground_mats.len(),
            cloud_mats.len(),
            buffers.len(),
            pipes.count()
        );
    }
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
