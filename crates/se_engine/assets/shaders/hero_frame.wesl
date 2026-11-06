// The Viltrumite mod's hero: the records shared by his draws (hero.wesl) and his light's compute pass
// (hero_light.wesl). No bindings here. Layout must match viltrumite/draw.rs `HeroFrameGpu`, `HeroLightGpu`.

import super::atmosphere_common::AtmParams;
import super::ground_light::{PatchLight, LightBody};

// One storage buffer, written in place each frame (never a material: a material changed every frame is
// prepared again).
struct HeroFrame {
    // Each joint's world * inverse bind, camera-relative: the matrix's three rows (the transform of his
    // meshes is the identity, so he is never compressed by the scaled space at any speed).
    joints: array<array<vec4<f32>, 3>, 128>,
    // xyz where his light is worked out (his chest, render frame), w unused.
    point: vec4<f32>,
    // xyz up at his place: away from the body he is near, else the camera's up.
    up: vec4<f32>,
    // rgb the street lamps' light at his place (the town's lights' colour times how far into a town he
    // is); 0 away from towns.
    town: vec4<f32>,
    // Later steps: the aerodynamic glow, the cape's wind, the plane his shadow falls on.
    heat: vec4<f32>,
    wind: vec4<f32>,
    shadow_plane: vec4<f32>,
    // Light from the further stars and shining planets where no body's light record is copied (the CPU's
    // path, in space): xyz towards it, w 1 when used; rgb its light, in the ground's scale. Taken with his
    // own normals, in the pixels. Near a body they are in `light`, as the ground takes them.
    free_dir: array<vec4<f32>, 3>,
    free_color: array<vec4<f32>, 3>,
    // What `point_light_at` reads: the same values the scatter's objects read.
    body: LightBody,
    atm: AtmParams,
    light: PatchLight,
}

// `point_light_at`'s result at `point`, written by hero_light.wesl before the main pass.
struct HeroLight {
    sun: vec4<f32>,
    sky: vec4<f32>,
    flat_e: vec4<f32>,
    haze_t: vec4<f32>,
    haze_a: vec4<f32>,
}
