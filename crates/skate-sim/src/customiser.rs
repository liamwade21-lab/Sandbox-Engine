//! Stand-in for upstream `customiser.rs` (the customiser UI). `apply_preferences` and `scalar` are
//! copied from it unchanged: the one place a profile (truck, wheel, gestures, stance, style,
//! posture) reaches the simulation.
use serde_json::Value;

fn scalar(profile: &Value, key: &str, initial: f64) -> f64 {
    profile
        .pointer(&format!("/{}", key.replace('.', "/")))
        .and_then(Value::as_f64)
        .unwrap_or(initial)
}

pub(crate) fn apply_preferences(
    profile: &Value,
    physics: &mut crate::physics::GamePhysics,
    animation: &mut crate::skater_animation::SkaterAnimation,
) {
    physics.set_equipment_preferences(
        scalar(profile, "truck", 0.7) as f32,
        scalar(profile, "wheel", 0.7) as f32,
    );
    physics.set_gesture_preferences(profile["gestures"].as_object().map(|g| {
        // ResetGestureSet824FA730 marks all 37 entries available and selects
        // the first four in table order for Up,Down,Left,Right.
        std::array::from_fn(|i| {
            g.get(&i.to_string())
                .and_then(Value::as_u64)
                .unwrap_or(i as u64) as u32
        })
    }));
    animation.set_customisation(
        scalar(profile, "stance", 1.) as u32,
        scalar(profile, "style", 0.) as u32,
    );
    animation
        .motion
        .animation
        .posture
        .set_profile(scalar(profile, "posture", 0.) as u32);
}
