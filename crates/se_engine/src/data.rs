//! Loads the SpaceEngine installation (virtual file system + catalogs).

use std::sync::Arc;

use bevy::prelude::*;
use se_core::catalog::Catalog;
use se_core::vfs::Vfs;

use crate::config::EngineConfig;

/// Shared, read-only game data.
#[derive(Resource, Clone)]
pub struct SeData {
    pub vfs: Arc<Vfs>,
    pub catalog: Arc<Catalog>,
}

impl SeData {
    /// Mutable catalog for adding objects at runtime. Everything that needs the catalog
    /// in a background task gets a snapshot of what it needs instead of a clone of this
    /// `Arc`, because a shared catalog is deep-copied here (a stall of a few hundred ms).
    pub fn catalog_mut(&mut self) -> &mut Catalog {
        if Arc::strong_count(&self.catalog) > 1 {
            let t = std::time::Instant::now();
            let cat = Arc::make_mut(&mut self.catalog);
            warn!("catalog shared: copied it in {:.0?}", t.elapsed());
            return cat;
        }
        Arc::make_mut(&mut self.catalog)
    }
}

/// Saved camera locations ("Places") from the SpaceEngine install.
#[derive(Resource, Default, Clone)]
pub struct Places(pub Vec<se_core::places::Place>);

/// The user's saved places: `%APPDATA%/SpaceEngine-rs/places.cfg`, never the install folder.
pub fn user_places_file() -> std::path::PathBuf {
    crate::settings::user_file("places.cfg")
}

/// Rewrites the user's places file with the user places in `places` (shipped ones are skipped).
/// Scripted runs (`--run`, `--screenshot`) write nothing, so test runs add no places.
pub fn write_user_places(cfg: &EngineConfig, places: &[se_core::places::Place]) -> std::io::Result<()> {
    if crate::settings::scripted(cfg) {
        return Ok(());
    }
    let path = user_places_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let user: Vec<se_core::places::Place> = places.iter().filter(|p| p.user).cloned().collect();
    // Temp file and rename, so a crash mid-write cannot leave the user's places empty.
    crate::settings::write_atomic(&path, &se_core::places::write_places(&user))
}

pub struct DataPlugin;

impl Plugin for DataPlugin {
    fn build(&self, app: &mut App) {
        let cfg = app.world().resource::<EngineConfig>().clone();
        let t = std::time::Instant::now();
        let vfs = match Vfs::mount_install(&cfg.se_path) {
            Ok(v) => v,
            Err(e) => {
                error!("cannot mount SpaceEngine data at {}: {e}", cfg.se_path.display());
                error!("pass --se-path <SpaceEngine dir> or set SE_PATH");
                std::process::exit(1);
            }
        };
        let encrypted: usize = vfs.archives().iter().map(|a| a.encrypted_entries).sum();
        info!(
            "mounted {} paks / {} files from {} ({} encrypted entries skipped) in {:.0?}",
            vfs.archives().len(),
            vfs.file_count(),
            cfg.se_path.display(),
            encrypted,
            t.elapsed()
        );
        let t = std::time::Instant::now();
        let mut catalog = Catalog::load(&vfs);
        // JPL DE436 for the major Solar System bodies, as SpaceEngine uses.
        let t_de = std::time::Instant::now();
        if let Some(path) = vfs.list_ext("ephemerides", "436").into_iter().next() {
            match vfs.read(&path).map(se_core::jpl_de::JplEphemeris::parse) {
                Ok(Ok(de)) => {
                    info!("ephemeris: {} ({:.0}..{:.0} JD) in {:.0?}", de.title, de.start_jd, de.end_jd, t_de.elapsed());
                    catalog.de = Some(Arc::new(de));
                }
                Ok(Err(e)) => warn!("ephemeris {path}: {e}"),
                Err(e) => warn!("ephemeris {path}: {e}"),
            }
        }
        // Analytic moon theories (GUST86, L1, TASS 1.7, ...): tables live in the executable.
        match se_core::moon_theories::MoonTheories::from_install(&cfg.se_path) {
            Ok(th) => catalog.moon_theories = Some(Arc::new(th)),
            Err(e) => warn!("moon theories unavailable ({e}); moons use catalog Kepler orbits"),
        }
        // Gas species and XUV tables for SE's completion of catalog planets.
        match se_core::climate::ClimateTables::from_install(&cfg.se_path) {
            Ok(t) => catalog.climate = Some(Arc::new(t)),
            Err(e) => warn!("climate tables unavailable ({e}); catalog planets keep only catalog atmospheres"),
        }
        // Orbit angles SE generates where the catalog leaves them out, and Msini / sin i masses.
        se_core::system_gen::complete_catalog_orbits(&mut catalog);
        info!(
            "catalogs: {} stars, {} bodies, {} galaxies, {} clusters, {} nebulae in {:.0?}",
            catalog.stars.len(),
            catalog.bodies.len(),
            catalog.galaxies.len(),
            catalog.clusters.len(),
            catalog.nebulae.len(),
            t.elapsed()
        );
        // SE's CatalogLogLevel / CsvLogLevel (log_file.rs) decide which loader messages are logged.
        let log = app.world().get_resource::<crate::log_file::CatalogLog>().map(|c| c.0).unwrap_or_default();
        for w in catalog.warnings.iter().filter(|w| log.logs_catalog(w.split(':').next().unwrap_or(""))).take(5) {
            warn!("catalog: {w}");
        }
        // Saved places shipped with SpaceEngine (plus the user's own, if present).
        let mut places = Vec::new();
        for file in ["save/places-def.cfg", "save/places.cfg"] {
            if let Ok(bytes) = std::fs::read(cfg.se_path.join(file)) {
                places.extend(se_core::places::parse_places(&se_core::vfs::decode_text(&bytes)));
            }
        }
        // The user's own places, saved by the Places window.
        if let Ok(bytes) = std::fs::read(user_places_file()) {
            let user = se_core::places::parse_places(&se_core::vfs::decode_text(&bytes));
            places.extend(user.into_iter().map(|p| se_core::places::Place { user: true, ..p }));
        }
        info!("{} saved places", places.len());
        app.insert_resource(Places(places));
        app.insert_resource(SeData {
            vfs: Arc::new(vfs),
            catalog: Arc::new(catalog),
        });
    }
}
