//! Derived weather (docs/spec-volumetric-clouds-weather.md, step 5): a pure, deterministic function
//! of a body's climate, a place on it, the star's direction and the date, so the same date always
//! gives the same weather and any date can be jumped to at once.
//!
//! It shapes the body's SE cloud cover (the engine's volumetric clouds multiply and offset it, never
//! paint over it):
//! - latitude belts of rising (wet) and sinking (dry) air: 3 per hemisphere for an Earth-like spin
//!   (the tropics, the subtropical highs, the storm tracks, the polar highs), more on faster
//!   rotators, one on slow ones, following the subsolar latitude with the seasons;
//! - mid-latitude storms along the wet belts, born, moving east with the westerlies and dying over a
//!   few days, with a comma-shaped cloud spiral;
//! - afternoon convection where the sun is high, and thunderstorms at night at half of it;
//! - large weather patterns evolving over days, carried by the belts' winds;
//! - fog on humid, calm nights and mornings, and dust storms on dry, windy worlds.
//!
//! Points are unit directions in the body frame (y the north pole, SE's planet frame).

use glam::DVec3;

/// What a body's weather depends on (built from its data by the engine).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    /// Per-body seed (the weather patterns and storms).
    pub seed: u64,
    /// Sidereal rotation period (hours; negative retrograde, 0 not turning).
    pub day_hours: f64,
    pub radius_m: f64,
    /// How wet the air is (0 dry .. 1 an ocean world).
    pub humidity: f64,
    pub tidal_lock: bool,
}

/// The weather at a place and time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Weather {
    /// The cloud cover becomes `clamp(cover * cover_mul + cover_add, 0, 1)`.
    pub cover_mul: f32,
    pub cover_add: f32,
    /// Convective activity (0..1): taller cumulus, thunderstorms.
    pub convection: f32,
    /// Precipitation (0..1).
    pub precip: f32,
    /// Wind towards the east and the north (m/s).
    pub wind: [f32; 2],
    /// Fog near the ground (0..1).
    pub fog: f32,
    /// Dust in the air (0..1).
    pub dust: f32,
}

const DAY_S: f64 = 86_400.0;
/// Storms live 5 days; the large patterns change over 1.5 days.
const STORM_LIFE_D: f64 = 5.0;
const PATTERN_D: f64 = 1.5;
/// Storms are about 1200 km across and 3500 km apart along their track.
const STORM_RADIUS_M: f64 = 1.2e6;
const STORM_SPACING_M: f64 = 3.5e6;

fn hash(a: u64, b: u64) -> u64 {
    let mut h = a ^ b.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= h >> 31;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth 3D value noise (-1..1).
fn vnoise(p: DVec3, seed: u64) -> f64 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (DVec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i64, i.y as i64, i.z as i64);
    let corner = |dx: i64, dy: i64, dz: i64| {
        let h = hash(hash(hash(seed, (x + dx) as u64), (y + dy) as u64), (z + dz) as u64);
        unit(h) * 2.0 - 1.0
    };
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z)
}

fn fbm(p: DVec3, seed: u64) -> f64 {
    (vnoise(p, seed) * 0.6 + vnoise(p * 2.1, seed ^ 1) * 0.3 + vnoise(p * 4.3, seed ^ 2) * 0.1) / 0.75
}

/// Rising/sinking-air cells per hemisphere: 3 at Earth's spin, more for faster rotators (Jupiter's
/// many bands), one for slow ones and tidally locked worlds.
pub fn cells(c: &Climate) -> u32 {
    if c.tidal_lock || c.day_hours == 0.0 {
        return 1;
    }
    (3.0 * (23.93 / c.day_hours.abs()).sqrt()).round().clamp(1.0, 7.0) as u32
}

/// The belt latitude: the latitude measured from the thermal equator, which follows the subsolar
/// latitude halfway.
fn belt_lat(lat: f64, sub_lat: f64) -> f64 {
    lat - 0.5 * sub_lat
}

/// The zonal wind speed scale (m/s): stronger on faster rotators.
fn wind_scale(c: &Climate) -> f64 {
    if c.day_hours == 0.0 {
        return 3.0;
    }
    (10.0 * (23.93 / c.day_hours.abs()).powf(0.3)).clamp(3.0, 60.0)
}

/// East-west wind (m/s) at belt latitude `b`: easterly trades, westerlies, polar easterlies.
fn zonal_wind(c: &Climate, b: f64) -> f64 {
    let n = cells(c) as f64;
    -wind_scale(c) * (2.0 * n * b.abs()).sin() * c.day_hours.signum()
}

fn lat_of(d: DVec3) -> f64 {
    d.y.clamp(-1.0, 1.0).asin()
}

/// East longitude in SE's planet frame (+X longitude 0, +Z 90 W).
fn lon_of(d: DVec3) -> f64 {
    (-d.z).atan2(d.x)
}

/// The point at latitude/east longitude.
pub fn dir_at(lat: f64, lon: f64) -> DVec3 {
    DVec3::new(lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin())
}

/// The weather at body-frame direction `p` (unit) when the star lies towards `sun` (body frame,
/// unit) at Julian date `jd`.
pub fn weather_at(c: &Climate, p: DVec3, sun: DVec3, jd: f64) -> Weather {
    let t_d = jd - crate::time::J2000;
    let lat = lat_of(p);
    let sub_lat = lat_of(sun);
    let b = belt_lat(lat, sub_lat);
    let n = cells(c) as f64;
    // Wet (+1) where air rises, dry (-1) where it sinks; poleward of 80 degrees always dry-ish.
    let belt = (2.0 * n * b).cos() * (1.0 - 0.5 * ((lat.abs() - 1.4).max(0.0) / 0.17).min(1.0));

    // The large patterns: noise slices each living PATTERN_D days, carried east by the belts' winds
    // from their own birth, blended across slices.
    let wind = zonal_wind(c, b);
    let slice_t = t_d / PATTERN_D;
    let s0 = slice_t.floor();
    let mut anomaly = 0.0;
    // Eddies on the belts' winds, carried with the patterns; where fog and dust gather.
    let mut eddy = [0.0f64; 2];
    let mut patch = [0.0f64; 2];
    for (k, w) in [(s0, 1.0 - (slice_t - s0)), (s0 + 1.0, slice_t - s0)] {
        let w = w * w * (3.0 - 2.0 * w);
        let age_s = (t_d - k * PATTERN_D) * DAY_S;
        let turn = wind * age_s / (c.radius_m * lat.cos().max(0.05));
        let q = DVec3::new(p.x * turn.cos() + p.z * turn.sin(), p.y, -p.x * turn.sin() + p.z * turn.cos());
        let qk = q * 3.0 + DVec3::splat(k * 7.31);
        let seed = c.seed ^ (k as i64 as u64);
        anomaly += w * fbm(qk, seed);
        eddy[0] += w * vnoise(qk * 0.7 + DVec3::splat(11.3), seed ^ 3);
        eddy[1] += w * vnoise(qk * 0.7 + DVec3::splat(23.7), seed ^ 4);
        patch[0] += w * fbm(qk * 1.7 + DVec3::splat(41.9), seed ^ 5);
        patch[1] += w * fbm(qk * 0.4 + DVec3::splat(57.1), seed ^ 6);
    }
    // The surface flow of the cells towards the rising belts (the trades converge on the thermal
    // equator, the middle latitudes' flow runs poleward).
    let meridional = -0.3 * wind_scale(c) * (2.0 * n * b).sin();

    // Storms along the wet belts away from the equator, in both hemispheres.
    let (mut storm_cover, mut storm_rain) = (0.0f64, 0.0f64);
    let r_ang = STORM_RADIUS_M / c.radius_m;
    let winter = |hemi: f64| 1.0 + 0.3 * (-hemi * sub_lat.sin() / 0.4).clamp(-1.0, 1.0);
    for m in 1..=(n as i32) {
        let track = m as f64 * std::f64::consts::PI / n;
        if track >= 1.45 {
            break;
        }
        for hemi in [1.0, -1.0] {
            let track_lat = hemi * track + 0.5 * sub_lat;
            if (lat - track_lat).abs() > 3.0 * r_ang + 0.2 {
                continue;
            }
            let slots = ((std::f64::consts::TAU * c.radius_m * track_lat.cos()) / STORM_SPACING_M).round().max(1.0) as u64;
            let u = zonal_wind(c, track_lat - 0.5 * sub_lat).abs().max(4.0) * c.day_hours.signum();
            for j in 0..slots {
                let hj = unit(hash(c.seed ^ 0x5707, (m as u64) << 32 | (hemi > 0.0) as u64 * 1000 + j));
                let gen_f = t_d / (STORM_LIFE_D * 0.5) + hj;
                for g in [gen_f.floor() - 1.0, gen_f.floor()] {
                    let born = (g - hj) * STORM_LIFE_D * 0.5;
                    let age = t_d - born;
                    if !(0.0..STORM_LIFE_D).contains(&age) {
                        continue;
                    }
                    let hg = hash(hash(c.seed, j), g as i64 as u64 ^ (m as u64) << 40);
                    let lon0 = (j as f64 + 0.5 * unit(hg)) / slots as f64 * std::f64::consts::TAU;
                    let clat = track_lat + (unit(hg >> 7) - 0.5) * 0.15;
                    let clon = lon0 + u * age * DAY_S / (c.radius_m * clat.cos().max(0.05));
                    let centre = dir_at(clat, clon);
                    let d = p.dot(centre).clamp(-1.0, 1.0).acos() / r_ang;
                    if d > 3.0 {
                        continue;
                    }
                    let life = (std::f64::consts::PI * age / STORM_LIFE_D).sin() * (0.6 + 0.4 * unit(hg >> 17)) * winter(hemi);
                    // The comma: a cloud arm spiralling into the low, the right way round for the
                    // hemisphere.
                    let east = DVec3::Y.cross(centre).normalize_or(DVec3::X);
                    let north = centre.cross(east);
                    let az = p.dot(north).atan2(p.dot(east));
                    let arm = 0.5 + 0.5 * (az - hemi * 2.5 * (d + 0.05).ln()).cos();
                    let f = (-d * d).exp();
                    storm_cover += life * f * (0.3 + 0.45 * arm);
                    storm_rain += life * f * arm;
                }
            }
        }
    }

    // Convection where the sun stands high: most in the afternoon (about 3 p.m. local), least in the
    // morning (under a tenth of it at 9 a.m.), and half of it at midnight (the night's storm systems,
    // the oceans' storms before dawn). (With the afternoon's alone the night side had no
    // thunderstorms: no lightning; at 40 % its storms stayed too weak to flash.)
    let hour = lon_of(p) - lon_of(sun);
    let afternoon = (hour - 0.7 * c.day_hours.signum()).cos().max(0.0);
    let night = 0.5 * (0.5 - 0.5 * hour.cos());
    let sun_high = (lat - sub_lat).cos().max(0.0).powi(2);
    let diurnal = if c.tidal_lock { p.dot(sun).max(0.0) } else { (afternoon * afternoon + night).min(1.0) * sun_high };
    let wet = 0.5 + 0.5 * belt;
    let convection = (diurnal * 0.8 * wet * c.humidity + storm_rain * 0.3).clamp(0.0, 1.0);

    let wind_e = wind + 0.4 * wind_scale(c) * eddy[0];
    let wind_n = meridional + 0.4 * wind_scale(c) * eddy[1];
    let speed = wind_e.hypot(wind_n);
    // Fog: humid, calm air at night and in the morning (most at dawn), over the sinking belts' cool
    // seas the most, in patches.
    let smooth = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let early = if c.tidal_lock { 1.0 - p.dot(sun).abs() } else { (hour + 1.4 * c.day_hours.signum()).cos().max(0.0) };
    let calm = 1.0 - smooth(2.0, 9.0, speed);
    // Fog banks, not a veil: a tenth or so of the calm, humid places.
    let fog = c.humidity * c.humidity * calm * (0.2 + 0.8 * early) * (0.5 + 0.5 * (-belt).max(0.0)) * (1.0 - convection)
        * smooth(0.3, 0.75, patch[0])
        * 2.5;
    // Dust: dry, windy worlds lift it in regional storms carried with the patterns (a world as wet
    // as Earth hardly ever).
    let dryness = 1.0 - c.humidity;
    let dust = dryness * dryness * (speed / 10.0).min(1.5) * smooth(0.2, 0.7, patch[1]) * 1.6;

    // The body's own cloud map (SE's) stays recognisable: the weather redistributes its cover gently
    // and storms thicken the clouds there rather than spread an overcast over clear sky (2026-11-03:
    // stronger terms turned Earth's storm track into a beige overcast seen from orbit).
    Weather {
        cover_mul: (1.0 + 0.2 * belt + 0.3 * anomaly + 0.45 * storm_cover).clamp(0.4, 1.8) as f32,
        // (Never below 0: subtracting wiped SE's thin clouds off the flat shells in the dry belts; the
        // multiplier thins them there instead.)
        cover_add: (0.08 * anomaly.max(0.0) + 0.2 * storm_cover).clamp(0.0, 0.5) as f32,
        convection: convection as f32,
        precip: ((storm_rain * 0.8 + convection * 0.5) * c.humidity.sqrt()).clamp(0.0, 1.0) as f32,
        wind: [wind_e as f32, wind_n as f32],
        fog: fog.clamp(0.0, 1.0) as f32,
        dust: dust.clamp(0.0, 1.0) as f32,
    }
}

/// The star's direction (body frame) `dt_s` seconds after it lay towards `sun`: it turns west with the
/// body's spin (the year's motion neglected), and stands still over a tidally locked world.
pub fn sun_after(c: &Climate, sun: DVec3, dt_s: f64) -> DVec3 {
    if c.tidal_lock || c.day_hours == 0.0 {
        return sun;
    }
    let a = -std::f64::consts::TAU * dt_s / (c.day_hours * 3600.0);
    DVec3::new(sun.x * a.cos() + sun.z * a.sin(), sun.y, -sun.x * a.sin() + sun.z * a.cos())
}

/// The wind map's range: a channel holds `wind / (2 WIND_RANGE) + 0.5` (m/s).
pub const WIND_RANGE: f32 = 100.0;

/// The weather map's size: an equirectangular map, texel (x, y) at east longitude
/// `(x + 0.5) / MAP_W * 2 pi - pi` and latitude `pi / 2 - (y + 0.5) / MAP_H * pi`.
pub const MAP_W: usize = 256;
pub const MAP_H: usize = 128;

/// The weather map (RGBA8, rows from the north), two layers one after the other: the first R the
/// cover multiplier / 2, G the cover offset x 0.5 + 0.5, B convection, A precipitation; the second
/// R, G the wind towards the east and the north (`WIND_RANGE`), B fog, A dust. Filled on all cores.
pub fn weather_map(c: &Climate, sun: DVec3, jd: f64) -> Vec<u8> {
    let layer = MAP_W * MAP_H * 4;
    let mut out = vec![0u8; layer * 2];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, MAP_H);
    let rows = MAP_H.div_ceil(threads);
    let (cover, wind) = out.split_at_mut(layer);
    std::thread::scope(|s| {
        for (k, (chunk, wchunk)) in cover.chunks_mut(rows * MAP_W * 4).zip(wind.chunks_mut(rows * MAP_W * 4)).enumerate() {
            s.spawn(move || {
                for (i, (px, wpx)) in chunk.chunks_exact_mut(4).zip(wchunk.chunks_exact_mut(4)).enumerate() {
                    let (x, y) = (i % MAP_W, k * rows + i / MAP_W);
                    let lon = (x as f64 + 0.5) / MAP_W as f64 * std::f64::consts::TAU - std::f64::consts::PI;
                    let lat = std::f64::consts::FRAC_PI_2 - (y as f64 + 0.5) / MAP_H as f64 * std::f64::consts::PI;
                    let w = weather_at(c, dir_at(lat, lon), sun, jd);
                    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let wb = |v: f32| b(v / (2.0 * WIND_RANGE) + 0.5);
                    px.copy_from_slice(&[b(w.cover_mul * 0.5), b(w.cover_add * 0.5 + 0.5), b(w.convection), b(w.precip)]);
                    wpx.copy_from_slice(&[wb(w.wind[0]), wb(w.wind[1]), b(w.fog), b(w.dust)]);
                }
            });
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth() -> Climate {
        Climate { seed: 42, day_hours: 23.93, radius_m: 6.371e6, humidity: 0.7, tidal_lock: false }
    }

    /// The star's direction at subsolar latitude `lat` over longitude 0.
    fn sun_at(lat_deg: f64) -> DVec3 {
        dir_at(lat_deg.to_radians(), 0.0)
    }

    /// The mean cover multiplier around a latitude circle.
    fn band_mul(c: &Climate, lat_deg: f64, sun: DVec3, jd: f64) -> f64 {
        (0..72).map(|i| weather_at(c, dir_at(lat_deg.to_radians(), i as f64 * 5f64.to_radians()), sun, jd).cover_mul as f64).sum::<f64>() / 72.0
    }

    /// The weather's effect on the cover, and fog and dust shares, for tuning:
    /// `cargo test -p se_core print_cover_stats -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_cover_stats() {
        let c = Climate { humidity: 0.8, ..earth() };
        let (s, jd) = (sun_at(-14.0), 2_461_344.32);
        for (name, cl) in [("earth", c), ("dry", Climate { humidity: 0.25, ..c })] {
            let all: Vec<Weather> = (0..2000).map(|i| {
                let (u, v) = ((i as f64 * 0.618034).fract(), (i as f64 + 0.5) / 2000.0);
                weather_at(&cl, dir_at((2.0 * v - 1.0).asin(), u * std::f64::consts::TAU), s, jd)
            }).collect();
            let share = |f: &dyn Fn(&Weather) -> bool| all.iter().filter(|w| f(w)).count() as f64 / all.len() as f64;
            println!("{name}: fog>0.3 {:.3} fog>0.1 {:.3} dust>0.3 {:.3} dust>0.1 {:.3}", share(&|w| w.fog > 0.3), share(&|w| w.fog > 0.1), share(&|w| w.dust > 0.3), share(&|w| w.dust > 0.1));
        }
        for lat in [-60, -40, -20, 0, 20, 40, 55, 70] {
            let ws: Vec<Weather> = (0..72).map(|i| weather_at(&c, dir_at((lat as f64).to_radians(), (i * 5) as f64 * 1f64.to_radians()), s, jd)).collect();
            let m = |f: &dyn Fn(&Weather) -> f64| ws.iter().map(f).sum::<f64>() / ws.len() as f64;
            let cov = |c0: f64| m(&|w| (c0 * w.cover_mul as f64 + w.cover_add as f64).clamp(0.0, 1.0));
            println!("lat {lat:4}: mul {:.2} add {:.2} | cover 0->{:.2} 0.2->{:.2} 0.4->{:.2} 0.6->{:.2} | fog {:.2} dust {:.2}", m(&|w| w.cover_mul as f64), m(&|w| w.cover_add as f64), cov(0.0), cov(0.2), cov(0.4), cov(0.6), m(&|w| w.fog as f64), m(&|w| w.dust as f64));
        }
    }

    #[test]
    fn same_inputs_same_weather_other_dates_differ() {
        let c = earth();
        let (p, s) = (dir_at(0.8, 1.3), sun_at(10.0));
        let jd = 2_461_000.3;
        assert_eq!(weather_at(&c, p, s, jd), weather_at(&c, p, s, jd));
        let later = weather_at(&c, p, s, jd + 2.0);
        assert_ne!(weather_at(&c, p, s, jd), later);
        // Another body (seed) has other weather.
        assert_ne!(weather_at(&Climate { seed: 7, ..c }, p, s, jd), weather_at(&c, p, s, jd));
    }

    #[test]
    fn latitude_belts_wet_tropics_dry_subtropics() {
        let c = earth();
        let (s, jd) = (sun_at(0.0), 2_461_000.0);
        let (itcz, subtrop, storms) = (band_mul(&c, 0.0, s, jd), band_mul(&c, 30.0, s, jd), band_mul(&c, 60.0, s, jd));
        assert!(itcz > subtrop + 0.25 && storms > subtrop + 0.25, "{itcz} {subtrop} {storms}");
        // Trade winds from the east, westerlies in the middle latitudes.
        let w = |lat: f64| weather_at(&c, dir_at(lat.to_radians(), 0.0), s, jd).wind[0];
        assert!(w(15.0) < 0.0 && w(45.0) > 0.0 && w(-45.0) > 0.0);
        // Faster spin, more belts.
        assert!(cells(&Climate { day_hours: 9.9, ..c }) > cells(&c) && cells(&Climate { day_hours: 2800.0, ..c }) == 1);
    }

    #[test]
    fn belts_follow_the_seasons() {
        let c = earth();
        let jd = 2_461_000.0;
        // The wet belt moves north with the northern summer sun: 12 N wetter in July than in January.
        let july = band_mul(&c, 12.0, sun_at(23.4), jd);
        let january = band_mul(&c, 12.0, sun_at(-23.4), jd);
        assert!(july > january + 0.2, "{july} {january}");
    }

    #[test]
    fn weather_map_matches_the_function() {
        let c = earth();
        let (s, jd) = (sun_at(5.0), 2_461_000.4);
        let map = weather_map(&c, s, jd);
        let layer = MAP_W * MAP_H * 4;
        assert_eq!(map.len(), layer * 2);
        assert_eq!(map, weather_map(&c, s, jd));
        // Texel (10, 40): its centre's weather, as stored.
        let (x, y) = (10usize, 40usize);
        let lon = (x as f64 + 0.5) / MAP_W as f64 * std::f64::consts::TAU - std::f64::consts::PI;
        let lat = std::f64::consts::FRAC_PI_2 - (y as f64 + 0.5) / MAP_H as f64 * std::f64::consts::PI;
        let w = weather_at(&c, dir_at(lat, lon), s, jd);
        let k = (y * MAP_W + x) * 4;
        assert!((map[k] as f32 / 255.0 - (w.cover_mul * 0.5).clamp(0.0, 1.0)).abs() < 0.003);
        assert!((map[k + 1] as f32 / 255.0 - (w.cover_add * 0.5 + 0.5)).abs() < 0.003);
        // The wind in the second layer.
        for (i, v) in w.wind.iter().enumerate() {
            assert!(((map[layer + k + i] as f32 / 255.0 - 0.5) * 2.0 * WIND_RANGE - v).abs() < 0.5, "{i} {v}");
        }
        // Longitude and latitude round-trip as the shader reads them.
        let d = dir_at(lat, lon);
        assert!(((-d.z).atan2(d.x) - lon).abs() < 1e-9 && (d.y.asin() - lat).abs() < 1e-9);
    }

    #[test]
    fn storms_move_east_and_afternoons_convect() {
        let c = earth();
        let s = sun_at(0.0);
        // Somewhere along the northern storm track there is a storm, and a day later it has moved east.
        let jd = 2_461_000.0;
        let track = 60f64.to_radians();
        // (The cover a 40% cloud map gets: storms thicken the clouds already there.)
        let cover = |lon_deg: f64, t: f64| {
            let w = weather_at(&c, dir_at(track, lon_deg.to_radians()), s, t);
            0.4 * w.cover_mul as f64 + w.cover_add as f64
        };
        let peak = (0..360).max_by(|&a, &b| cover(a as f64, jd).total_cmp(&cover(b as f64, jd))).unwrap() as f64;
        assert!(cover(peak, jd) > 0.6, "{}", cover(peak, jd));
        let moved = (0..60).map(|k| peak + k as f64).max_by(|a, b| cover(*a, jd + 1.0).total_cmp(&cover(*b, jd + 1.0))).unwrap();
        assert!(moved > peak + 3.0, "{peak} -> {moved}");
        // At the equator, convection peaks in the afternoon (east of the subsolar meridian), not at dawn.
        let conv = |lon_deg: f64| weather_at(&c, dir_at(0.0, lon_deg.to_radians()), s, jd).convection;
        assert!(conv(40.0) > conv(-80.0) + 0.2);
    }

    #[test]
    fn fog_on_calm_humid_mornings_dust_on_dry_worlds() {
        let c = earth();
        let s = sun_at(0.0);
        let jd = 2_461_000.0;
        // Around a latitude circle: fog only where it is night or morning, never in the afternoon.
        let at = |c: &Climate, lon_deg: f64| weather_at(c, dir_at(25f64.to_radians(), lon_deg.to_radians()), s, jd);
        let mean = |c: &Climate, f: &dyn Fn(&Weather) -> f32, lons: std::ops::Range<i32>| {
            lons.clone().map(|l| f(&at(c, l as f64)) as f64).sum::<f64>() / lons.len() as f64
        };
        let dawn = mean(&c, &|w| w.fog, -120..-60);
        let afternoon = mean(&c, &|w| w.fog, 30..60);
        assert!(dawn > 0.02 && afternoon < dawn * 0.3, "{dawn} {afternoon}");
        // Earth hardly lifts dust; a dry world does, in regional storms (a part of the globe, not a veil).
        let dry = Climate { humidity: 0.1, ..c };
        let share = |c: &Climate, over: f32| {
            let n = 2000;
            let hits = (0..n)
                .filter(|&i| {
                    let (u, v) = ((i as f64 * 0.618034).fract(), (i as f64 + 0.5) / n as f64);
                    weather_at(c, dir_at((2.0 * v - 1.0).asin(), u * std::f64::consts::TAU), s, jd).dust > over
                })
                .count();
            hits as f64 / n as f64
        };
        assert!(share(&c, 0.1) < 0.01, "{}", share(&c, 0.1));
        let storms = share(&dry, 0.1);
        assert!(storms > 0.04 && storms < 0.3, "{storms}");
    }

    #[test]
    fn trades_converge_and_the_sun_turns_west() {
        let c = earth();
        let (s, jd) = (sun_at(0.0), 2_461_000.0);
        // Around the latitude circle the trades blow towards the equator from both sides.
        let v = |lat: f64| (0..72).map(|i| weather_at(&c, dir_at(lat.to_radians(), ((i * 5) as f64).to_radians()), s, jd).wind[1] as f64).sum::<f64>() / 72.0;
        assert!(v(15.0) < -1.0 && v(-15.0) > 1.0, "{} {}", v(15.0), v(-15.0));
        // Six hours later the sun stands 90 degrees further west; a locked world's sun stays.
        let later = sun_after(&c, dir_at(0.0, 0.0), 6.0 * 3600.0 * c.day_hours / 24.0);
        assert!((later - dir_at(0.0, -std::f64::consts::FRAC_PI_2)).length() < 1e-9, "{later}");
        let locked = Climate { tidal_lock: true, ..c };
        assert_eq!(sun_after(&locked, s, 3600.0), s);
    }
}
