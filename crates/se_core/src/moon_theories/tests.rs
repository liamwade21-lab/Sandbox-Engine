//! Tests for the moon theories.
//!
//! The unit tests need no install. The `#[ignore]` tests read `SpaceEngine.exe`
//! (set `SE_PATH`, default `D:/SteamLibrary/steamapps/common/SpaceEngine`) and are run
//! with `cargo test -p se_core moon_theories -- --ignored`.

use super::*;
use crate::coords::eq_to_universe;
use crate::units::METERS_PER_AU;

fn install() -> std::path::PathBuf {
    std::env::var_os("SE_PATH")
        .map(Into::into)
        .unwrap_or_else(|| "D:/SteamLibrary/steamapps/common/SpaceEngine".into())
}

fn theories() -> MoonTheories {
    MoonTheories::from_install(&install()).expect("SpaceEngine.exe readable")
}

/// Offset in J2000 ecliptic km (x = equinox, z = north), the frame the executable works in.
fn ecliptic_km(th: &MoonTheories, model: &str, jd: f64) -> [f64; 3] {
    let [x, y, z] = th.internal_au(model, jd).unwrap_or_else(|| panic!("{model} unsupported"));
    let k = METERS_PER_AU / 1000.0;
    [x * k, -z * k, y * k]
}

/// Minimal PE with one section at RVA 0x1000 (0x100 bytes virtual, 0x10 backed by the file).
fn tiny_pe(with_section: bool) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    b[0..2].copy_from_slice(b"MZ");
    b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    b[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
    b[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    b[0xb0..0xb8].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    if with_section {
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        let so = 0x80 + 24 + 0xf0;
        b[so + 8..so + 12].copy_from_slice(&0x100u32.to_le_bytes());
        b[so + 12..so + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[so + 16..so + 20].copy_from_slice(&0x10u32.to_le_bytes());
        b[so + 20..so + 24].copy_from_slice(&0x300u32.to_le_bytes());
        b[0x300..0x308].copy_from_slice(&2.5f64.to_le_bytes());
        b[0x308..0x310].copy_from_slice(&7u64.to_le_bytes());
    }
    b
}

/// Position of moons relative to their planet from JPL Horizons (geometric, TDB,
/// ICRF equatorial axes, km), with the tolerance (km) we expect for SpaceEngine's theory.
const HORIZONS: &[(&str, f64, [f64; 3], f64)] = &[
    ("Io-L1", 2451545.0, [399714.236, 114358.234, 61202.667], 300.0),
    ("Io-L1", 2458849.5, [335830.665, -231931.737, -105336.979], 300.0),
    ("Ganymede-L1", 2451545.0, [-821345.095, -615085.673, -304338.121], 400.0),
    ("Ganymede-L1", 2458849.5, [-972572.421, -401091.303, -206862.891], 400.0),
    ("Mimas-TASS17", 2451545.0, [140987.947, -116421.225, -2082.948], 2700.0),
    ("Mimas-TASS17", 2458849.5, [-186615.403, 24256.346, 11658.516], 2700.0),
    ("Titan-TASS17", 2451545.0, [-946802.938, 824098.225, 27082.230], 300.0),
    ("Titan-TASS17", 2458849.5, [-1237873.811, 193646.239, 96793.223], 300.0),
    ("Rhea-TASS17", 2451545.0, [-524762.378, -23096.960, 49541.717], 300.0),
    ("Rhea-TASS17", 2458849.5, [-490516.923, 192390.682, 25602.277], 300.0),
    ("Iapetus-TASS17", 2451545.0, [-2853773.239, -2271215.095, 160569.278], 1100.0),
    ("Iapetus-TASS17", 2458849.5, [-1582856.098, -3244233.587, -279111.757], 1100.0),
    ("Ariel-GUST86", 2451545.0, [175677.936, -19272.292, -72960.037], 1500.0),
    ("Ariel-GUST86", 2458849.5, [23319.295, 45885.028, -183876.059], 1500.0),
    ("Titania-GUST86", 2451545.0, [-63107.002, 128000.323, -411987.194], 4200.0),
    ("Titania-GUST86", 2458849.5, [-143103.060, 139381.588, -386810.177], 4200.0),
    ("Oberon-GUST86", 2451545.0, [-560569.960, 144960.989, -72810.140], 5400.0),
    ("Oberon-GUST86", 2458849.5, [563818.852, -96138.881, -108708.758], 5400.0),
    ("Phobos-NOE-4", 2451545.0, [-1988.978, -8743.160, -3182.262], 400.0),
    ("Phobos-NOE-4", 2458849.5, [-2218.612, 7606.269, 5049.705], 400.0),
    ("Deimos-NOE-4", 2451545.0, [10366.440, -15747.664, -13945.346], 500.0),
    ("Deimos-NOE-4", 2458849.5, [12066.351, -14383.566, -14069.420], 500.0),
];

/// Positions (J2000 ecliptic km) produced by the executable's own evaluation
/// functions for the same models, obtained by running them in an x86-64 emulator.
/// They pin the port to the executable's behaviour, including its quirks.
const EXE_PARITY: &[(&str, f64, [f64; 3])] = &[
    ("Io-L1", 2451545.0, [396725.743, 128184.300, 10576.690]),
    ("Io-L1", 2458849.5, [333478.299, -253144.204, -4365.701]),
    ("Titan-TASS17", 2451545.0, [-946986.248, 766788.010, -302920.111]),
    ("Titan-TASS17", 2458849.5, [-1237998.821, 216029.045, 11846.152]),
    ("Hyperion-TASS17", 2451545.0, [172377.543, 1275481.180, -658925.569]),
    ("Hyperion-TASS17", 2458849.5, [-1605032.995, 461033.422, -81089.199]),
    ("Titania-GUST86", 2451545.0, [-65300.590, -45242.673, -428688.297]),
    ("Titania-GUST86", 2458849.5, [-146038.548, -25180.205, -409351.830]),
    ("Phobos-NOE-4", 2451545.0, [-1975.916, -9158.508, 417.542]),
    ("Phobos-NOE-4", 2458849.5, [-1946.013, 9044.809, 1536.649]),
    ("Telesto-HTC20", 2451545.0, [281708.426, 63786.563, -60843.936]),
    ("Telesto-HTC20", 2458849.5, [-188337.277, 209350.650, -87988.472]),
    ("Janus-Noyelles", 2451545.0, [143891.871, 36413.594, -33835.053]),
    ("Janus-Noyelles", 2458849.5, [-66609.995, 122362.588, -56149.154]),
    ("Epimetheus-Noyelles", 2451545.0, [-149528.442, -23335.696, 26710.921]),
    ("Epimetheus-Noyelles", 2458849.5, [84801.326, -112466.110, 50718.751]),
    ("Triton-Emelyanov", 2451545.0, [293681.042, 40214.779, -194905.492]),
    ("Triton-Emelyanov", 2458849.5, [312965.150, 146333.759, -80580.269]),
];

#[test]
fn matrix_product_uses_row_convention() {
    let a = rot_z(0.3);
    let b = rot_y(-1.1);
    let v = [0.2, -0.7, 1.3];
    let ab = mat_mul(&a, &b);
    // (v * A) * B == v * (A * B)
    let step = row_apply(row_apply(v, &a), &b);
    let once = row_apply(v, &ab);
    for i in 0..3 {
        assert!((step[i] - once[i]).abs() < 1e-14);
    }
    // Rotations preserve length.
    let n = |p: [f64; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    assert!((n(once) - n(v)).abs() < 1e-14);
}

#[test]
fn circular_orbit_lies_in_internal_xz_plane() {
    // The orbit normal is the internal Y axis: the third component carries -sin(lambda).
    for lam in [0.0f64, 0.7, 2.0, 4.0] {
        let p = elliptic_position(2.0, [lam, 0.0, 0.0, 0.0, 0.0]);
        assert!((p[0] - 2.0 * lam.cos()).abs() < 1e-12);
        assert!(p[1].abs() < 1e-12);
        assert!((p[2] + 2.0 * lam.sin()).abs() < 1e-12);
    }
}

#[test]
fn eccentric_orbit_has_kepler_radius_distribution() {
    // k = e cos(varpi), h = e sin(varpi): e = 0.5. The radius spans [a(1-e), a(1+e)]
    // and its mean over mean longitude is a (1 + e^2 / 2).
    let (k, h) = (0.3, 0.4);
    let n = 3600;
    let (mut lo, mut hi, mut sum) = (f64::MAX, 0.0f64, 0.0);
    for i in 0..n {
        let lam = std::f64::consts::TAU * i as f64 / n as f64;
        let p = elliptic_position(1.0, [lam, k, h, 0.0, 0.0]);
        let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        lo = lo.min(r);
        hi = hi.max(r);
        sum += r;
    }
    assert!((lo - 0.5).abs() < 1e-4 && (hi - 1.5).abs() < 1e-4, "{lo} {hi}");
    assert!((sum / n as f64 - 1.125).abs() < 1e-4);
}

#[test]
fn inclination_tilts_the_orbit_normal() {
    // q = sin(i/2) with the node on the x axis (p = 0): a quarter orbit after the
    // node the body is displaced by a sin(i) along the internal Y (normal) axis.
    let i: f64 = 0.4;
    let q = (i / 2.0).sin();
    let p = elliptic_position(1.0, [std::f64::consts::FRAC_PI_2, 0.0, 0.0, q, 0.0]);
    assert!((p[1].abs() - i.sin()).abs() < 1e-12, "{p:?}");
}

#[test]
fn exe_image_reads_mapped_data_only() {
    let exe = ExeImage::parse(tiny_pe(true)).unwrap();
    assert_eq!(exe.f64(0x1_4000_1000), Some(2.5));
    assert_eq!(exe.u64(0x1_4000_1008), Some(7));
    assert_eq!(exe.f64(0x1_4000_1010), None, "beyond the raw data");
    assert_eq!(exe.f64(0x1_3fff_0000), None, "below the image");
    assert!(ExeImage::parse(vec![0; 64]).is_err());
}

#[test]
fn unloaded_theories_are_reported_unsupported() {
    let th = MoonTheories::from_exe(&ExeImage::parse(tiny_pe(false)).unwrap());
    assert!(!th.supports("Io-L1"));
    assert!(!th.supports("Titan-TASS17"));
    assert!(th.offset_m("Titan-TASS17", 2451545.0).is_none());
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn matches_horizons() {
    // The L1 entries are checked without SpaceEngine's Chebyshev correction, which
    // moves the Galilean moons by thousands of km (see `l1_chebyshev_effect`).
    let th = theories().with_l1_chebyshev(false);
    let mut worst = 0.0f64;
    for &(model, jd, hz, tol) in HORIZONS {
        let got = th.offset_m(model, jd).unwrap_or_else(|| panic!("{model} unsupported"));
        let want = eq_to_universe(DVec3::from_array(hz) * 1000.0);
        let err_km = (got - want).length() / 1000.0;
        worst = worst.max(err_km);
        assert!(err_km < tol, "{model} @ {jd}: {err_km:.0} km (limit {tol} km)");
    }
    eprintln!("worst error {worst:.0} km");
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn matches_executable_output() {
    let th = theories();
    for &(model, jd, want) in EXE_PARITY {
        let got = ecliptic_km(&th, model, jd);
        let d = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2) + (got[2] - want[2]).powi(2)).sqrt();
        assert!(d < 0.05, "{model} @ {jd}: off by {d:.4} km: {got:?} vs {want:?}");
    }
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn l1_chebyshev_effect() {
    let jd = 2451545.0;
    let on = theories();
    let off = theories().with_l1_chebyshev(false);
    for model in ["Io-L1", "Europa-L1", "Ganymede-L1", "Callisto-L1"] {
        let a = on.offset_m(model, jd).unwrap();
        let b = off.offset_m(model, jd).unwrap();
        let shift_km = (a - b).length() / 1000.0;
        assert!((2000.0..15000.0).contains(&shift_km), "{model}: {shift_km:.0} km");
    }
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn validity_ranges_and_support() {
    let th = theories();
    assert!(th.supports("Janus-Noyelles") && th.supports("mimas-tass17") && th.supports("Triton-Emelyanov"));
    assert!(!th.supports("Io-TASS17") && !th.supports("Moon-EMB-DE") && !th.supports("Earth"));
    // Noyelles is only valid for 1950..2050.
    assert!(th.offset_m("Janus-Noyelles", 2451545.0).is_some());
    assert!(th.offset_m("Janus-Noyelles", 2433000.0).is_none());
    assert!(th.offset_m("Janus-Noyelles", 2470000.0).is_none());
    // L1 is valid for 1550..2350, TASS is unbounded.
    assert!(th.offset_m("Io-L1", 2287000.0).is_none());
    assert!(th.offset_m("Titan-TASS17", 2287000.0).is_some());
    // Orbit sizes: Titan 1.2 million km, Phobos 9400 km, Miranda 130 000 km.
    let r = |m: &str| th.offset_m(m, 2455000.0).unwrap().length() / 1000.0;
    assert!((1.0e6..1.4e6).contains(&r("Titan-TASS17")));
    assert!((9000.0..9800.0).contains(&r("Phobos-NOE-4")));
    assert!((125_000.0..135_000.0).contains(&r("Miranda-GUST86")));
}

#[test]
#[ignore = "reads the SpaceEngine install"]
fn ephemeris_uses_theories_for_catalog_moons() {
    use crate::catalog::Catalog;
    use crate::ephemeris::Ephemeris;
    use std::sync::Arc;
    let vfs = crate::vfs::Vfs::mount_install(&install()).expect("mount");
    let mut cat = Catalog::load(&vfs);
    let jd = 2458849.5;
    // Designations, since plain "Io" also names an asteroid.
    let moons = [
        ("Saturn VI", "Saturn", "Titan-TASS17"),
        ("Jupiter I", "Jupiter", "Io-L1"),
        ("Mars I", "Mars", "Phobos-NOE-4"),
        ("Uranus IV", "Uranus", "Oberon-GUST86"),
    ];
    // Without the theories the catalog's Keplerian orbits are used.
    let kepler: Vec<f64> = {
        let eph = Ephemeris::new(&cat);
        moons
            .iter()
            .map(|(m, p, _)| eph.position(cat.find(m).unwrap(), jd).delta_m(eph.position(cat.find(p).unwrap(), jd)).length())
            .collect()
    };
    let th = Arc::new(theories());
    cat.moon_theories = Some(th.clone());
    let eph = Ephemeris::new(&cat);
    for (i, (m, p, model)) in moons.iter().enumerate() {
        let d = eph.position(cat.find(m).unwrap(), jd).delta_m(eph.position(cat.find(p).unwrap(), jd));
        let want = th.offset_m(model, jd).unwrap();
        assert!((d - want).length() < 1.0, "{m}: {} m", (d - want).length());
        // Same ballpark as the Keplerian orbit but not identical.
        assert!((d.length() / kepler[i] - 1.0).abs() < 0.2, "{m}: {} vs {}", d.length(), kepler[i]);
    }
}
