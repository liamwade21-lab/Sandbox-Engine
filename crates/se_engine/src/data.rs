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
    /// Marks the catalog changed ([`Catalog::touch`]) for the caches of catalog data.
    pub fn catalog_mut(&mut self) -> &mut Catalog {
        let cat = if Arc::strong_count(&self.catalog) > 1 {
            let t = std::time::Instant::now();
            let cat = Arc::make_mut(&mut self.catalog);
            warn!("catalog shared: copied it in {:.0?}", t.elapsed());
            cat
        } else {
            Arc::make_mut(&mut self.catalog)
        };
        cat.touch();
        cat
    }

    /// The catalog to change in place, if nothing else holds it (no deep copy), marked changed.
    pub fn catalog_get_mut(&mut self) -> Option<&mut Catalog> {
        let cat = Arc::get_mut(&mut self.catalog)?;
        cat.touch();
        Some(cat)
    }

    /// Which catalog this is and its [`Catalog::revision`]: caches of catalog data rebuild when it
    /// differs (the `Arc` too, as a test or a session can swap the whole catalog).
    pub fn catalog_key(&self) -> (usize, u64) {
        (Arc::as_ptr(&self.catalog) as usize, self.catalog.revision())
    }
}

/// Saved camera locations ("Places") from the SpaceEngine install.
#[derive(Resource, Default, Clone)]
pub struct Places(pub Vec<se_core::places::Place>);

/// The user's saved places: `%APPDATA%/SpaceEngine-rs-v2/places.cfg`, never the install folder.
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

/// The install (VFS, catalogs, ephemeris) loading on a thread started at the top of `main`, so it
/// overlaps Bevy's own plugin start-up (the wgpu device, pipelines); [`DataPlugin`] joins it.
pub struct DataLoad(std::thread::JoinHandle<Loaded>);

impl DataLoad {
    pub fn start(cfg: &EngineConfig) -> Self {
        let se_path = cfg.se_path.clone();
        let thread = std::thread::Builder::new().name("se data load".into()).spawn(move || load(&se_path));
        Self(thread.expect("cannot start the data load thread"))
    }
}

/// What the load thread made, with the log lines it would have printed (the logger is not set up
/// while it runs), in order.
struct Loaded {
    log: Vec<(LogLevel, String)>,
    /// `None`: the install could not be mounted.
    data: Option<(Vfs, Catalog)>,
}

enum LogLevel {
    Info,
    Warn,
    Error,
    /// Vfs::mount_dir's own warning for a pak it cannot read.
    PakWarn,
}

fn load(se_path: &std::path::Path) -> Loaded {
    let mut log = Vec::new();
    let t = std::time::Instant::now();
    let vfs = match Vfs::mount_install(se_path) {
        Ok(v) => v,
        Err(e) => {
            log.push((LogLevel::Error, format!("cannot mount SpaceEngine data at {}: {e}", se_path.display())));
            log.push((LogLevel::Error, "pass --se-path <SpaceEngine dir> or set SE_PATH".into()));
            return Loaded { log, data: None };
        }
    };
    let elapsed = t.elapsed();
    log.extend(skipped_paks(se_path, &vfs).into_iter().map(|w| (LogLevel::PakWarn, w)));
    let encrypted: usize = vfs.archives().iter().map(|a| a.encrypted_entries).sum();
    log.push((
        LogLevel::Info,
        format!(
            "mounted {} paks / {} files from {} ({} encrypted entries skipped) in {:.0?}",
            vfs.archives().len(),
            vfs.file_count(),
            se_path.display(),
            encrypted,
            elapsed
        ),
    ));
    let t = std::time::Instant::now();
    let (mut catalog, de) = std::thread::scope(|s| {
        // JPL DE436 for the major Solar System bodies, as SpaceEngine uses. Inflating the 102 MB
        // file takes about half as long as the catalogs, so it is read alongside them.
        let de = s.spawn(|| {
            let t_de = std::time::Instant::now();
            let path = vfs.list_ext("ephemerides", "436").into_iter().next()?;
            let de = vfs.read(&path).map(se_core::jpl_de::JplEphemeris::parse);
            Some((path, de, t_de.elapsed()))
        });
        (Catalog::load(&vfs), de.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
    });
    if let Some((path, de, elapsed)) = de {
        match de {
            Ok(Ok(de)) => {
                log.push((LogLevel::Info, format!("ephemeris: {} ({:.0}..{:.0} JD) in {:.0?}", de.title, de.start_jd, de.end_jd, elapsed)));
                catalog.de = Some(Arc::new(de));
            }
            Ok(Err(e)) => log.push((LogLevel::Warn, format!("ephemeris {path}: {e}"))),
            Err(e) => log.push((LogLevel::Warn, format!("ephemeris {path}: {e}"))),
        }
    }
    // Analytic moon theories (GUST86, L1, TASS 1.7, ...): tables live in the executable.
    match se_core::moon_theories::MoonTheories::from_install(se_path) {
        Ok(th) => catalog.moon_theories = Some(Arc::new(th)),
        Err(e) => log.push((LogLevel::Warn, format!("moon theories unavailable ({e}); moons use catalog Kepler orbits"))),
    }
    // Gas species and XUV tables for SE's completion of catalog planets.
    match se_core::climate::ClimateTables::from_install(se_path) {
        Ok(t) => catalog.climate = Some(Arc::new(t)),
        Err(e) => log.push((LogLevel::Warn, format!("climate tables unavailable ({e}); catalog planets keep only catalog atmospheres"))),
    }
    // Orbit angles SE generates where the catalog leaves them out, and Msini / sin i masses.
    se_core::system_gen::complete_catalog_orbits(&mut catalog);
    log.push((
        LogLevel::Info,
        format!(
            "catalogs: {} stars, {} bodies, {} galaxies, {} clusters, {} nebulae in {:.0?}",
            catalog.stars.len(),
            catalog.bodies.len(),
            catalog.galaxies.len(),
            catalog.clusters.len(),
            catalog.nebulae.len(),
            t.elapsed()
        ),
    ));
    Loaded { log, data: Some((vfs, catalog)) }
}

/// The "skipping pak" warnings `Vfs::mount_install` logged (lost on the load thread): the paks
/// it walked past in its own order (each directory's, sorted) that are not mounted.
fn skipped_paks(se_path: &std::path::Path, vfs: &Vfs) -> Vec<String> {
    let mut out = Vec::new();
    let addons = se_path.join("addons");
    for root in std::iter::once(se_path.join("data")).chain(addons.is_dir().then_some(addons)) {
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            let mut paks = Vec::new();
            for item in read.flatten() {
                let path = item.path();
                if item.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(path);
                } else if item.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".pak") {
                    paks.push(path);
                }
            }
            paks.sort();
            for pak in paks.into_iter().filter(|p| !vfs.archives().iter().any(|a| a.path == *p)) {
                if let Err(e) = Vfs::new().mount_pak(&pak, "") {
                    out.push(format!("skipping pak {}: {e}", pak.display()));
                }
            }
        }
    }
    out
}

/// Inserts [`SeData`] and [`Places`]: joins the [`DataLoad`] started in `main` (or loads now).
#[derive(Default)]
pub struct DataPlugin(std::sync::Mutex<Option<DataLoad>>);

impl DataPlugin {
    pub fn new(load: DataLoad) -> Self {
        Self(std::sync::Mutex::new(Some(load)))
    }
}

impl Plugin for DataPlugin {
    fn build(&self, app: &mut App) {
        let cfg = app.world().resource::<EngineConfig>().clone();
        let load = self.0.lock().unwrap().take().unwrap_or_else(|| DataLoad::start(&cfg));
        let Loaded { log, data } = load.0.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
        for (level, line) in log {
            match level {
                LogLevel::Info => info!("{line}"),
                LogLevel::Warn => warn!("{line}"),
                LogLevel::Error => error!("{line}"),
                LogLevel::PakWarn => warn!(target: "se_core::vfs", "{line}"),
            }
        }
        let Some((vfs, catalog)) = data else { std::process::exit(1) };
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
