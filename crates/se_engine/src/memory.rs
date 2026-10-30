//! SE's memory settings (main-def.cfg `Memory`) and its memory report in the log (seMemoryManager,
//! FUN_14030b0e0 and FUN_14030aae0).
//!
//! The cache sizes drive the port's matching caches: `CacheSizeSolarSystems` the generated
//! planetary systems kept in the catalog (system.rs, least recently entered dropped first),
//! `CacheSizeLandNode` the terrain nodes of all planets together (terrain.rs, least recently used
//! first, at most the port's own ceiling), `CacheSizeStarNode` and `CacheSizeGalNode` the procedural star and galaxy octree nodes
//! (procstars.rs, procgalaxies.rs), `CacheSizeGalSysModels` the generated star clusters and nebulae
//! of galaxies (procsystems.rs) and `CacheSizeAtmoModels` the atmosphere models (atmosphere.rs).
//!
//! Not ported: the RAM and VRAM caps themselves (`VideoMemoryStrategy`, `VideoMemorySize`,
//! `VideoMemoryReserved`, `RAMSizeMax`, `RAMSizeMaxPercentage`) only set what the log reports, as the
//! port has no memory manager that budgets its allocations against them; `CacheSizeLandTextures`
//! (SE's terrain texture array; the port's surface textures are per-file), `CacheSizeGalModels` and
//! the `CacheSize*ModelsVRAM` minimums (the port draws a fixed few galaxy and nebula models,
//! galaxies.rs and nebulae.rs `max_models`, with no resident model cache behind them).

use bevy::prelude::*;

use crate::config::EngineConfig;

pub struct MemoryPlugin;

impl Plugin for MemoryPlugin {
    fn build(&self, app: &mut App) {
        let limits = app.world().get_resource::<EngineConfig>().map(MemoryLimits::load).unwrap_or_default();
        app.insert_resource(limits).add_systems(Startup, (log_memory, size_atmosphere_cache));
    }
}

fn size_atmosphere_cache(limits: Res<MemoryLimits>, cache: Option<ResMut<crate::atmosphere::AtmosphereCache>>) {
    if let Some(mut c) = cache {
        c.set_cap(limits.atmo_models);
    }
}

/// main-def.cfg `Memory`.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct MemoryLimits {
    /// `VideoMemoryStrategy`: 0 the VRAM limit is `VideoMemorySize`, 1 the larger of it and the
    /// physical VRAM, 2 the free VRAM (NVidia only).
    pub video_strategy: u8,
    /// `VideoMemorySize` and `VideoMemoryReserved` (MB).
    pub video_size_mb: u64,
    pub video_reserved_mb: u64,
    /// `RAMSizeMax` (MB) and `RAMSizeMaxPercentage` (of the physical RAM).
    pub ram_max_mb: u64,
    pub ram_max_percent: u64,
    /// `CacheSizeSolarSystems`: planetary systems kept in memory.
    pub solar_systems: usize,
    /// `CacheSizeLandNode`, `CacheSizeStarNode`, `CacheSizeGalNode`: octree / quadtree nodes kept.
    pub land_nodes: usize,
    pub star_nodes: usize,
    pub gal_nodes: usize,
    /// `CacheSizeGalSysModels`: galaxy subsystems (their star clusters and nebulae) kept.
    pub gal_sys_models: usize,
    /// `CacheSizeAtmoModels`: atmosphere models kept (0: no limit).
    pub atmo_models: usize,
}

impl Default for MemoryLimits {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self {
            video_strategy: 1,
            video_size_mb: 3072,
            video_reserved_mb: 50,
            ram_max_mb: 8192,
            ram_max_percent: 85,
            solar_systems: 64,
            land_nodes: 10000,
            star_nodes: 50000,
            gal_nodes: 10000,
            gal_sys_models: 32,
            atmo_models: 0,
        }
    }
}

impl MemoryLimits {
    /// Reads the `Memory` block of SE's main-def.cfg text; missing keys keep SE's defaults. The node
    /// caches' 0 ("auto") keeps the default too (SE's automatic sizes are not decoded).
    pub fn from_main_def(text: &str) -> Self {
        let doc = se_core::sc::parse(text);
        let d = Self::default();
        let num = |k: &str| doc.find_deep(k).and_then(|i| i.f64()).filter(|v| *v >= 0.0);
        let int = |k: &str, def: u64| num(k).map_or(def, |v| v as u64);
        let nodes = |k: &str, def: usize| num(k).filter(|v| *v >= 1.0).map_or(def, |v| v as usize);
        Self {
            video_strategy: int("VideoMemoryStrategy", d.video_strategy as u64).min(2) as u8,
            video_size_mb: int("VideoMemorySize", d.video_size_mb),
            video_reserved_mb: int("VideoMemoryReserved", d.video_reserved_mb),
            ram_max_mb: int("RAMSizeMax", d.ram_max_mb),
            ram_max_percent: int("RAMSizeMaxPercentage", d.ram_max_percent).min(100),
            solar_systems: nodes("CacheSizeSolarSystems", d.solar_systems),
            land_nodes: nodes("CacheSizeLandNode", d.land_nodes),
            star_nodes: nodes("CacheSizeStarNode", d.star_nodes),
            gal_nodes: nodes("CacheSizeGalNode", d.gal_nodes),
            gal_sys_models: nodes("CacheSizeGalSysModels", d.gal_sys_models),
            atmo_models: int("CacheSizeAtmoModels", d.atmo_models as u64) as usize,
        }
    }

    pub fn load(cfg: &EngineConfig) -> Self {
        std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).map_or_else(|_| Self::default(), |t| Self::from_main_def(&t))
    }

    /// SE's "Max SE RAM" (MB): `RAMSizeMaxPercentage` of the physical RAM, at most `RAMSizeMax`.
    pub fn max_ram_mb(&self, total_mb: u64) -> u64 {
        (total_mb * self.ram_max_percent / 100).min(self.ram_max_mb)
    }

    /// SE's "Max SE VRAM" (MB) and whether the config forced it: strategy 0 takes `VideoMemorySize`,
    /// the others the larger of it and the physical VRAM when that is known.
    pub fn max_vram_mb(&self, physical_mb: Option<u64>) -> (u64, bool) {
        match physical_mb {
            Some(p) if self.video_strategy != 0 && p > 0 => (p.max(self.video_size_mb), false),
            _ => (self.video_size_mb, self.video_strategy == 0),
        }
    }

    /// SE's memory report, line by line (FUN_14030b0e0, FUN_14030aae0).
    pub fn report(&self, ram: Option<(u64, u64)>, vram_mb: Option<u64>) -> Vec<String> {
        let mut out = Vec::new();
        if let Some((total, avail)) = ram {
            out.push(format!("Total      RAM: {total} Mb"));
            out.push(format!("Available  RAM: {avail} Mb"));
            out.push(format!("Max SE     RAM: {} Mb", self.max_ram_mb(total)));
        }
        out.push(match vram_mb {
            Some(v) => format!("Physical  VRAM: {v} Mb"),
            None => "Physical  VRAM: UNSUPPORTED".to_string(),
        });
        // SE asks the driver for the free VRAM only with strategy 2 (GL_NVX_gpu_memory_info); the
        // port has no such query.
        out.push(format!("Available VRAM: {}", if self.video_strategy == 2 { "UNSUPPORTED" } else { "DISABLED" }));
        let (max, forced) = self.max_vram_mb(vram_mb);
        out.push(format!("Max SE    VRAM: {max} Mb {}", if forced { "(forced by config)" } else { "(max of physical and config)" }));
        out.push(format!("System OS VRAM: {} Mb (from config)", self.video_reserved_mb));
        out
    }
}

/// The entries a cache of `len` entries drops to get back to `cap`: the least recently used
/// (`last_used`, larger is later) of those that may go.
pub fn lru_victims<K>(len: usize, cap: usize, entries: impl Iterator<Item = (K, u64)>) -> Vec<K> {
    if len <= cap {
        return Vec::new();
    }
    let mut v: Vec<(K, u64)> = entries.collect();
    v.sort_by_key(|e| e.1);
    v.into_iter().take(len - cap).map(|e| e.0).collect()
}

/// Physical RAM: total and available (MB), as SE reads them (GlobalMemoryStatusEx).
#[cfg(windows)]
fn ram_mb() -> Option<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut m = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: `m` is a MEMORYSTATUSEX with its length set, which is all the call writes to.
    let ok = unsafe { GlobalMemoryStatusEx(&mut m) } != 0;
    ok.then_some((m.total_phys >> 20, m.avail_phys >> 20))
}

#[cfg(not(windows))]
fn ram_mb() -> Option<(u64, u64)> {
    None
}

/// Writes SE's memory report to the log, from a worker (the VRAM query starts a `reg` process).
fn log_memory(limits: Res<MemoryLimits>) {
    let limits = limits.clone();
    std::thread::spawn(move || {
        for line in limits.report(ram_mb(), crate::graphics::video_memory_mb()) {
            info!("{line}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_main_def_and_reports_like_se() {
        let m = MemoryLimits::from_main_def(
            "Memory\n{\n\tVideoMemoryStrategy 0\n\tVideoMemorySize 4096\n\tRAMSizeMax 4000\n\tCacheSizeSolarSystems 8\n\tCacheSizeLandNode 0\n\tCacheSizeAtmoModels 12\n}\n",
        );
        assert_eq!((m.video_strategy, m.video_size_mb, m.ram_max_mb, m.solar_systems), (0, 4096, 4000, 8));
        // 0 ("auto") keeps the default node cache.
        assert_eq!((m.land_nodes, m.atmo_models, m.star_nodes), (10000, 12, 50000));
        assert_eq!(MemoryLimits::from_main_def(""), MemoryLimits::default());
        let d = MemoryLimits::default();
        // 85% of 16 GB is above RAMSizeMax 8192.
        assert_eq!(d.max_ram_mb(16303), 8192);
        assert_eq!(d.max_ram_mb(8000), 6800);
        assert_eq!(d.max_vram_mb(Some(16303)), (16303, false));
        assert_eq!(d.max_vram_mb(Some(2048)), (3072, false));
        assert_eq!(m.max_vram_mb(Some(16303)), (4096, true));
        let r = d.report(Some((16303, 9000)), Some(16303));
        assert_eq!(r[0], "Total      RAM: 16303 Mb");
        assert_eq!(r[2], "Max SE     RAM: 8192 Mb");
        assert_eq!(r[4], "Available VRAM: DISABLED");
        assert_eq!(r[5], "Max SE    VRAM: 16303 Mb (max of physical and config)");
        assert_eq!(r[6], "System OS VRAM: 50 Mb (from config)");
    }

    #[test]
    fn lru_drops_the_oldest() {
        let e = [("a", 5), ("b", 1), ("c", 9), ("d", 3)];
        assert_eq!(lru_victims(6, 4, e.into_iter()), vec!["b", "d"]);
        assert!(lru_victims(4, 4, e.into_iter()).is_empty());
        // Only what may go is dropped, even when that leaves the cache over its size.
        assert_eq!(lru_victims(10, 4, e.into_iter()).len(), 4);
    }
}
