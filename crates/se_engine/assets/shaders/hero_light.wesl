// The Viltrumite mod's hero: his light for this frame (viltrumite/draw.rs), one compute invocation a frame
// before the main pass, through the same function the scatter's trees and buildings use (ground_light.wesl
// `point_light_at`). Once a frame instead of per vertex: it reads about 30 textures, and he has 165,706
// vertices. Only his own draws read the result.

import super::eclipse::eclipse_light;
import super::ground_light::{CloudShadow, point_light_at, cloud_shadow_at};
import super::hero_frame::{HeroFrame, HeroLight};

@group(0) @binding(0) var<storage, read> frame: HeroFrame;
@group(0) @binding(1) var<storage, read_write> out_light: HeroLight;
@group(0) @binding(2) var inscatter_tex: texture_3d<f32>;
@group(0) @binding(3) var transmittance_tex: texture_2d<f32>;
@group(0) @binding(4) var irradiance_tex: texture_2d<f32>;
@group(0) @binding(5) var atm_smp: sampler;
@group(0) @binding(6) var cloud_cover: texture_cube<f32>;
@group(0) @binding(7) var cloud_cover_smp: sampler;
@group(0) @binding(8) var<storage, read> cloud_sh: CloudShadow;
@group(0) @binding(9) var cloud_map: texture_2d<f32>;
@group(0) @binding(10) var cloud_map_smp: sampler;
@group(0) @binding(11) var cloud_weather: texture_cube<f32>;
@group(0) @binding(12) var cloud_pattern: texture_cube<f32>;

@compute @workgroup_size(1)
fn light() {
    // Only the clouds above him shade him: the record was written for the ground under the camera. A layer
    // fades as he climbs through it; past the highest the shadow is off, and with it the shadow map (its
    // shapes are the ground's, seen through every layer). The layers' heights are the sphere frame's: on a
    // flattened body the radial distance reads up to flattening x radius too low (Earth 21 km at the poles).
    var sh = cloud_sh;
    let b = frame.body;
    let rel = frame.point.xyz - b.centre.xyz;
    let local = vec3<f32>(dot(rel, b.m0.xyz) / dot(b.m0.xyz, b.m0.xyz), dot(rel, b.m1.xyz) / dot(b.m1.xyz, b.m1.xyz), dot(rel, b.m2.xyz) / dot(b.m2.xyz, b.m2.xyz));
    let height = length(local) - b.centre.w;
    var above = false;
    for (var i = 0; i < 4; i++) {
        let a = sh.layer_a[i];
        if a.z > 0.0 {
            sh.layer_a[i].z = a.z * (1.0 - smoothstep(a.x, a.y, height));
            above = above || sh.layer_a[i].z > 0.0;
        }
    }
    if !above {
        sh.key.z = 0.0;
    }
    let pl = point_light_at(
        frame.body,
        frame.atm,
        frame.light,
        inscatter_tex,
        transmittance_tex,
        irradiance_tex,
        atm_smp,
        sh,
        cloud_cover,
        cloud_cover_smp,
        cloud_map,
        cloud_map_smp,
        cloud_weather,
        cloud_pattern,
        frame.point.xyz,
    );
    // How much of the star's direct light clouds and an eclipse let through here, for his shadow: they take the sky light in the same
    // proportion (`point_light_at`), so the light alone doesn't say. The same call and arguments as its own.
    let direct = cloud_shadow_at(sh, cloud_cover, cloud_cover_smp, cloud_map, cloud_map_smp, cloud_weather, cloud_pattern, b.centre.w, b.key.xy, normalize(local), length(rel) - b.centre.w)
        * dot(eclipse_light(frame.light.eclipse[0], rel / b.centre.w, normalize(frame.light.sun_dir.xyz)), vec3<f32>(1.0 / 3.0));
    let down = frame.point.w;
    out_light = HeroLight(vec4<f32>(pl.sun * down, direct), vec4<f32>(pl.sky * down, 0.0), vec4<f32>(pl.flat_e * down, 0.0), vec4<f32>(pl.haze_t, 0.0), vec4<f32>(pl.haze_a, 0.0));
}
