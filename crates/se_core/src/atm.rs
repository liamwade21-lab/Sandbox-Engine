//! Precomputed atmospheric scattering tables (`models/atmospheres/*.atm`).
//!
//! Each file is a 60-byte header of 15 little-endian `f32` followed by three
//! RGBA32F tables in the layout of Bruneton & Neyret 2008 "Precomputed
//! Atmospheric Scattering":
//!
//! | table         | dimensions (w x h x d)                  | floats     |
//! |---------------|-----------------------------------------|------------|
//! | inscatter     | (MU_S 32 * NU 16) x MU 64 x R 32        | 4 194 304  |
//! | transmittance | MU 256 x R 64                           | 65 536     |
//! | irradiance    | MU_S 64 x R 16                          | 4 096      |
//!
//! Header: `Rg, Rt, HR, HM, betaR.rgb, betaMSca.rgb, betaMEx.rgb, mieG, mieFade`
//! (radii and heights in km, betas in 1/km). Table order and ranges were verified
//! by value statistics (transmittance peaks at exactly 1.0).

use thiserror::Error;

pub const RES_R: usize = 32;
// SE's own resolutions (`ag_common.glh`, `atmo_common.glh`: RES_MU 64, RES_NU 16, not Bruneton's 128/8).
pub const RES_MU: usize = 64;
pub const RES_MU_S: usize = 32;
pub const RES_NU: usize = 16;
pub const TRANSMITTANCE_W: usize = 256;
pub const TRANSMITTANCE_H: usize = 64;
pub const SKY_W: usize = 64;
pub const SKY_H: usize = 16;

pub const HEADER_FLOATS: usize = 15;
pub const INSCATTER_FLOATS: usize = RES_MU_S * RES_NU * RES_MU * RES_R * 4;
pub const TRANSMITTANCE_FLOATS: usize = TRANSMITTANCE_W * TRANSMITTANCE_H * 4;
pub const IRRADIANCE_FLOATS: usize = SKY_W * SKY_H * 4;
pub const FILE_SIZE: usize = (HEADER_FLOATS + INSCATTER_FLOATS + TRANSMITTANCE_FLOATS + IRRADIANCE_FLOATS) * 4;

#[derive(Debug, Error)]
pub enum AtmError {
    #[error("unexpected .atm size {0} (expected {FILE_SIZE})")]
    BadSize(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtmParams {
    /// Ground radius, km.
    pub radius_ground: f32,
    /// Top of atmosphere radius, km.
    pub radius_top: f32,
    pub rayleigh_h: f32,
    pub mie_h: f32,
    pub rayleigh_beta: [f32; 3],
    pub mie_sca_beta: [f32; 3],
    pub mie_ext_beta: [f32; 3],
    pub mie_g: f32,
    pub mie_fade: f32,
}

#[derive(Debug, Clone)]
pub struct AtmTables {
    pub params: AtmParams,
    /// RGBA, x = nu * RES_MU_S + mu_s (fastest), then mu, then r.
    pub inscatter: Vec<f32>,
    pub transmittance: Vec<f32>,
    pub irradiance: Vec<f32>,
}

impl AtmTables {
    /// The tables as an `.atm` file (header, inscatter, transmittance, irradiance).
    pub fn to_bytes(&self) -> Vec<u8> {
        let p = &self.params;
        let header = [
            p.radius_ground, p.radius_top, p.rayleigh_h, p.mie_h, p.rayleigh_beta[0], p.rayleigh_beta[1], p.rayleigh_beta[2], p.mie_sca_beta[0], p.mie_sca_beta[1],
            p.mie_sca_beta[2], p.mie_ext_beta[0], p.mie_ext_beta[1], p.mie_ext_beta[2], p.mie_g, p.mie_fade,
        ];
        header.iter().chain(&self.inscatter).chain(&self.transmittance).chain(&self.irradiance).flat_map(|f| f.to_le_bytes()).collect()
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, AtmError> {
        if bytes.len() != FILE_SIZE {
            return Err(AtmError::BadSize(bytes.len()));
        }
        let floats: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let h = &floats[..HEADER_FLOATS];
        let params = AtmParams {
            radius_ground: h[0],
            radius_top: h[1],
            rayleigh_h: h[2],
            mie_h: h[3],
            rayleigh_beta: [h[4], h[5], h[6]],
            mie_sca_beta: [h[7], h[8], h[9]],
            mie_ext_beta: [h[10], h[11], h[12]],
            mie_g: h[13],
            mie_fade: h[14],
        };
        let mut o = HEADER_FLOATS;
        let inscatter = floats[o..o + INSCATTER_FLOATS].to_vec();
        o += INSCATTER_FLOATS;
        let transmittance = floats[o..o + TRANSMITTANCE_FLOATS].to_vec();
        o += TRANSMITTANCE_FLOATS;
        let irradiance = floats[o..o + IRRADIANCE_FLOATS].to_vec();
        Ok(Self {
            params,
            inscatter,
            transmittance,
            irradiance,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_matches_observed_files() {
        assert_eq!(FILE_SIZE, 17_055_804);
    }

    #[test]
    fn parse_synthetic() {
        let mut bytes = vec![0u8; FILE_SIZE];
        bytes[0..4].copy_from_slice(&6360.0f32.to_le_bytes());
        bytes[4..8].copy_from_slice(&6420.0f32.to_le_bytes());
        let t = AtmTables::parse(&bytes).unwrap();
        assert_eq!(t.params.radius_ground, 6360.0);
        assert_eq!(t.params.radius_top, 6420.0);
        assert_eq!(t.inscatter.len(), INSCATTER_FLOATS);
        assert!(AtmTables::parse(&bytes[1..]).is_err());
        assert_eq!(t.to_bytes(), bytes);
    }
}
