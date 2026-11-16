//! The ship's assembly (T3b): `ship.json` (`se_core::elite_ship::Ship`), from the decoded export's `animation/` (the rigs
//! and every state machine of their files, sound cues' events checked against `index.json`), `paintjobs.json` (the
//! default paint job's record `DefaultPaintJob`), `models/sidewinder.json` (the hull's camera and hardpoint bones) and
//! what `re/elite/notes/sidewinder_paintjob.md` says of the ship item `SideWinder`. Values are Elite's (grade A); nothing
//! is converted to the mod's space. The rules that play it are `se_core::elite_rig`. The rigs are `models::RIGS` (the
//! drive bolt-ons are the arena's and left out: `models::NOT_SHIPPED`).
//!
//! `build` works the file out from the export alone; `convert` writes it, `verify` compares the folder's with a fresh
//! build without writing; `check` tests it against the shipped models.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value};

use se_core::elite_asset::parse_edm;
use se_core::elite_ship::{
    parse_ship, write_ship, Activity, CueEvents, GearAttach, Item, KnownActivity, Machine, Paint, PaintKey, Rig, RigNode, Ship, KNOWN_ACTIVITIES, PAINT_KEYS,
    SHIP_FORMAT,
};

use super::models::{exported_models, read_json, resolve_model, stock_rig, RIGS};
use super::{Ctx, Shipped};

/// What the notes say of the ship item, and the note.
const ITEM_NOTE: &str = "re/elite/notes/sidewinder_paintjob.md (the ship item SideWinder, item database 201ee494..., and its prototype 86ec03fb...)";

fn obj<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a Map<String, Value>, String> {
    v.get(key).and_then(Value::as_object).ok_or_else(|| format!("{file}: no table {key}"))
}

fn text<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a str, String> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| format!("{file}: no text {key}"))
}

/// A decoded rig node in `ship.json`'s shape: the model as a file name (a hash matched to an exported model), Elite's
/// reference kept, the motions without the export's derived `axis_name` and `sign`.
fn node(n: &Value, exported: &[String], file: &str) -> Result<Value, String> {
    let mut n = n.as_object().cloned().ok_or_else(|| format!("{file}: a rig node is not a table"))?;
    let model_ref = n.get("model").and_then(Value::as_str).map(str::to_string);
    let model = model_ref.as_deref().map(|m| resolve_model(m, exported, file)).transpose()?;
    n.insert("model".into(), model.map_or(Value::Null, Value::String));
    n.insert("model_ref".into(), model_ref.map_or(Value::Null, Value::String));
    for m in n.get_mut("motions").and_then(Value::as_array_mut).into_iter().flatten() {
        if let Some(m) = m.as_object_mut() {
            m.remove("axis_name");
            m.remove("sign");
        }
    }
    let kids: Vec<Value> = n.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
    let kids = kids.iter().map(|c| node(c, exported, file)).collect::<Result<Vec<_>, _>>()?;
    n.insert("children".into(), Value::Array(kids));
    Ok(Value::Object(n))
}

/// A point light's +0x88 (the specular scale) from the export's raw dump: the f32 at byte 136 (a word the dump leaves
/// out is zero).
fn specular_scale(raw: &[Value]) -> f64 {
    raw.iter()
        .find(|r| r.get("at").and_then(Value::as_u64) == Some(136))
        .and_then(|r| r.get("f32")?.get(0)?.as_f64())
        .unwrap_or(0.0)
}

/// A decoded activity in `ship.json`'s shape: the raw dump dropped, but a light's kept whole (its intensity animator's
/// fields are only there) with a point light's specular scale read from it.
fn activity(a: &Value, file: &str) -> Result<Activity, String> {
    let mut a = a.as_object().cloned().ok_or_else(|| format!("{file}: an activity is not a table"))?;
    let raw = a.remove("raw");
    a.remove("_ptr");
    let kind = a.get("type").and_then(Value::as_str).unwrap_or("");
    if matches!(kind, "PointLightActivity" | "SpotLightActivity") {
        let raw = raw.ok_or_else(|| format!("{file}: a light without its raw dump"))?;
        if kind == "PointLightActivity" {
            a.insert("specular_scale".into(), specular_scale(raw.as_array().map_or(&[][..], Vec::as_slice)).into());
        }
        a.insert("raw".into(), raw);
    }
    let kind = a.get("type").and_then(Value::as_str).unwrap_or("?").to_string();
    let a = Value::Object(a);
    let why = |e: serde_json::Error| format!("{file}: activity {kind}: {e}");
    if KNOWN_ACTIVITIES.contains(&kind.as_str()) {
        serde_json::from_value(a).map(Activity::Known).map_err(why)
    } else {
        serde_json::from_value(a).map(Activity::Other).map_err(why)
    }
}

/// Every state machine of a file, its states in the export's order (`index.json`'s lists), each sound cue's events
/// checked against `index.json`'s.
fn machines(j: &Value, index: &Value, file: &str, owner: &str) -> Result<Vec<Machine>, String> {
    let listed = index.get("state_machines").and_then(Value::as_object).ok_or_else(|| format!("index.json: no {file}"))?;
    let cues = index.get("sound_cues").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for sm in obj(j, "state_machines", file)?.values() {
        let stored = text(sm, "name", file)?;
        let name = stored.split_once("::").map_or(stored, |(_, n)| n);
        let order = listed.get(stored).and_then(|m| m.get("states")).and_then(Value::as_array).ok_or_else(|| format!("index.json: {file} has no {stored}"))?;
        let states = obj(sm, "states", file)?;
        let mut st_out = Vec::new();
        for sn in order {
            let sn = sn.as_str().ok_or_else(|| format!("index.json: {stored}'s states"))?;
            let st = states.get(sn).ok_or_else(|| format!("{file}: {stored} has no state {sn}"))?;
            let acts = st.get("activities").and_then(Value::as_array).ok_or_else(|| format!("{file}: {sn} has no activities"))?;
            let acts = acts.iter().map(|a| activity(a, file)).collect::<Result<Vec<_>, _>>()?;
            for a in &acts {
                let Activity::Known(KnownActivity::KinematicBezierCurveMultiple(c)) = a else { continue };
                for q in &c.sound_cues {
                    let entry = cues.iter().find(|x| {
                        x.get("state_machine").and_then(Value::as_str) == Some(stored)
                            && x.get("state").and_then(Value::as_str) == Some(sn)
                            && x.get("cue").and_then(Value::as_str) == Some(q.cue.as_str())
                    });
                    let events = entry.and_then(|x| x.get("events")).ok_or_else(|| format!("index.json: no cue {} in {stored} {sn}", q.cue))?;
                    let events: CueEvents = serde_json::from_value(events.clone()).map_err(|e| format!("index.json: cue {}: {e}", q.cue))?;
                    if events != q.events {
                        return Err(format!("{file} {stored} {sn}: cue {}'s events differ from index.json's", q.cue));
                    }
                }
            }
            let transitions = st.get("transitions").cloned().unwrap_or(Value::Array(vec![]));
            let mut s = Map::new();
            s.insert("name".into(), sn.into());
            s.insert("activities".into(), serde_json::to_value(&acts).map_err(|e| e.to_string())?);
            s.insert("transitions".into(), transitions);
            st_out.push(Value::Object(s));
        }
        let m = serde_json::json!({
            "name": name, "stored_name": stored, "owner": owner, "file": file,
            "initial": text(sm, "initial", file)?, "states": st_out,
        });
        out.push(serde_json::from_value(m).map_err(|e| format!("{file} {stored}: {e}"))?);
    }
    Ok(out)
}

/// The default paint job's record.
fn paint(dec: &Path) -> Result<Paint, String> {
    let file = "paintjobs.json";
    let j = read_json(&dec.join(file))?;
    let job = j.get("paintjob_sidewinder_default").ok_or_else(|| format!("{file}: no paintjob_sidewinder_default"))?;
    let rec = job.get("variants").and_then(|v| v.get("DefaultPaintJob")).ok_or_else(|| format!("{file}: no DefaultPaintJob"))?;
    let tex = text(job.get("texture").unwrap_or(&Value::Null), "file", file)?;
    let tex_name = tex.rsplit('/').next().unwrap_or(tex);
    let keys = obj(rec, "keys", file)?;
    let keys = PAINT_KEYS
        .iter()
        .map(|k| {
            let mut v = keys.get(*k).cloned().ok_or_else(|| format!("{file}: no key {k}"))?;
            v.as_object_mut().ok_or("a key is not a table")?.insert("name".into(), (*k).into());
            serde_json::from_value::<PaintKey>(v).map_err(|e| format!("{file}: key {k}: {e}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut p = rec.as_object().cloned().ok_or("the record is not a table")?;
    p.remove("raw_hex");
    p.remove("number_check");
    p.insert("keys".into(), serde_json::to_value(keys).map_err(|e| e.to_string())?);
    p.insert("paint_job".into(), "paintjob_sidewinder_default".into());
    p.insert("source".into(), job.get("source").cloned().unwrap_or(Value::Null));
    p.insert("folder".into(), job.get("folder").cloned().unwrap_or(Value::Null));
    p.insert("texture_file".into(), format!("tex/{tex_name}").into());
    serde_json::from_value(Value::Object(p)).map_err(|e| format!("{file}: DefaultPaintJob: {e}"))
}

/// What the ship item and its prototype say, with the hull's camera, hardpoint and utility bones.
fn item(dec: &Path, rigs: &[Rig]) -> Result<Item, String> {
    let file = "models/sidewinder.json";
    let j = read_json(&dec.join(file))?;
    let bones: Vec<&str> = j.get("bones").and_then(Value::as_array).ok_or("models/sidewinder.json: no bones")?.iter().filter_map(|b| b.get("name")?.as_str()).collect();
    let named = |what: &str| -> Vec<String> { bones.iter().filter(|b| b.to_lowercase().contains(what)).map(|b| b.to_string()).collect() };
    let cameras = bones.iter().filter(|b| b.starts_with("!Camera_Ship_")).map(|b| format!("Sidewinder{b}")).collect();
    let gear = ["Front", "Left", "Right"]
        .iter()
        .map(|side| {
            let rig = rigs.iter().find(|r| r.name.eq_ignore_ascii_case(&format!("Ext_LandingGear_SideWinder_{side}"))).ok_or_else(|| format!("no gear rig {side}"))?;
            Ok(GearAttach { rig: rig.name.clone(), node: "LandingGearRoot".into(), bone: format!("!AttachLandingGear{side}") })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sources = [
        ("item, item_path, hull_model, cockpit_model, seats, schematic_model", ITEM_NOTE.to_string()),
        ("shadow_model", format!("{ITEM_NOTE}: the prototype")),
        ("chair_models", "re/elite/notes/sidewinder_paintjob.md, \"The chair\" (CockpitChair -> Cockpit_Chair; LowPolyCockpitChair -> Ext_Cockpit_Chair)".to_string()),
        ("gear", format!("{ITEM_NOTE}: the rigs Ext_LandingGear_SideWinder_* at LandingGearRoot!AttachLandingGear*, each side's own bone")),
        ("cameras", format!("{ITEM_NOTE}: Sidewinder!Camera_Ship_*, each the hull's bone of that name (decoded models/sidewinder.json)")),
        ("hardpoints, utility_mounts", "the hull's bones (decoded models/sidewinder.json) whose names say so".to_string()),
    ];
    Ok(Item {
        item: "SideWinder".into(),
        item_path: "Items/Ships/SideWinder/SideWinder".into(),
        hull_model: "sidewinder".into(),
        cockpit_model: "sidewinder_cockpit".into(),
        seats: vec!["Sidewinder_Cockpit!Pilot_Chair".into(), "Sidewinder!Pilot_Chair".into()],
        chair_models: vec!["cockpit_chair".into(), "ext_cockpit_chair".into()],
        gear,
        cameras,
        schematic_model: "sidewinder_schematic".into(),
        shadow_model: "shadow_sidewinder".into(),
        hardpoints: named("hardpoint"),
        utility_mounts: named("utility"),
        sources: sources.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    })
}

/// `ship.json` from the export alone, with the export files it comes from.
fn build(dec: &Path) -> Result<(Ship, Vec<String>), String> {
    let exported = exported_models(dec)?;
    let index = read_json(&dec.join("animation/index.json"))?;
    let (mut rigs, mut all) = (Vec::new(), Vec::new());
    let mut sources = vec!["decoded/animation/index.json".to_string()];
    for (file, which) in RIGS {
        let rel = format!("animation/{file}.json");
        let j = read_json(&dec.join(&rel))?;
        let source = text(&j, "source", &rel)?;
        let r = stock_rig(&j, *which, &rel)?;
        let name = text(r, "name", &rel)?;
        let root: RigNode = serde_json::from_value(node(r.get("root").ok_or_else(|| format!("{rel}: no root"))?, &exported, &rel)?)
            .map_err(|e| format!("{rel}: rig {name}: {e}"))?;
        rigs.push(Rig {
            name: name.to_string(),
            file: file.to_string(),
            source: source.to_string(),
            category: r.get("category").and_then(Value::as_str).map(str::to_string),
            root,
        });
        let idx = index.get(*file).ok_or_else(|| format!("index.json: no {file}"))?;
        all.extend(machines(&j, idx, file, name)?);
        sources.push(format!("decoded/{rel} (Elite's {source})"));
    }
    let paint = paint(dec)?;
    sources.push(format!("decoded/paintjobs.json (Elite's {})", paint.source));
    let item = item(dec, &rigs)?;
    sources.push("decoded/models/sidewinder.json (the bone names)".into());
    sources.push(ITEM_NOTE.into());
    Ok((Ship { format: SHIP_FORMAT, rigs, machines: all, paint, item }, sources))
}

/// The folder's `ship.json` is what a fresh build makes. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let (ship, _) = build(&ctx.export)?;
    let bytes = ctx.read("ship.json")?;
    if parse_ship(&bytes).map_err(|e| e.to_string())? != ship || bytes != write_ship(&ship) {
        return Err("ship.json differs from what the decoded files give".into());
    }
    check(ctx, &bytes)?;
    println!("ship.json: {} rigs, {} machines match the export", ship.rigs.len(), ship.machines.len());
    Ok(())
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let (ship, sources) = build(&ctx.export)?;
    let bytes = write_ship(&ship);
    ctx.write("ship.json", &bytes)?;
    let back = parse_ship(&ctx.read("ship.json")?).map_err(|e| e.to_string())?;
    if back != ship {
        return Err("ship.json: what was read back differs from what was written".into());
    }
    shipped.add(
        "ship.json",
        &sources,
        "A",
        "The assembly: the stock ship's rigs, every state machine of their files with typed activities, the default paint \
         job, what the ship item says. Elite's values and model space.",
    );
    Ok(())
}

/// The read-back's checks of `ship.json` against the rest of the folder: every model a rig node names is in `models/`,
/// every attach bone is in its parent's model (case-insensitive, as the engine matches), the gear's attach points are
/// on the hull rig, every value a machine drives is a rig motion's, every light's bones and the paint job's mask exist.
pub(super) fn check(ctx: &Ctx, bytes: &[u8]) -> Result<(), String> {
    let ship = parse_ship(bytes).map_err(|e| e.to_string())?;
    let mut bones: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut model_bones = |model: &str| -> Result<BTreeSet<String>, String> {
        let model = model.to_lowercase();
        if !bones.contains_key(&model) {
            let rel = format!("models/{model}.edm");
            let edm = parse_edm(&ctx.read(&rel)?).map_err(|e| format!("{rel}: {e}"))?;
            bones.insert(model.clone(), edm.parts.iter().map(|p| p.name.to_lowercase()).collect());
        }
        Ok(bones[&model].clone())
    };
    let mut motions = BTreeSet::new();
    for rig in &ship.rigs {
        let mut stack: Vec<(&RigNode, Option<&str>)> = vec![(&rig.root, None)];
        while let Some((n, parent_model)) = stack.pop() {
            if let Some(m) = &n.model {
                model_bones(m).map_err(|e| format!("rig {}, node {}: {e}", rig.name, n.name))?;
            }
            if let (Some(bone), Some(pm)) = (&n.attach_bone, parent_model) {
                if !model_bones(pm)?.contains(&bone.to_lowercase()) {
                    return Err(format!("rig {}, node {}: no bone {bone} in models/{pm}.edm", rig.name, n.name));
                }
            }
            motions.extend(n.motions.iter().map(|m| m.value.as_str()));
            stack.extend(n.children.iter().map(|c| (c, n.model.as_deref())));
        }
    }
    let hull = ship.rig(&ship.item.hull_model).ok_or("no hull rig")?;
    for g in &ship.item.gear {
        ship.rig(&g.rig).ok_or_else(|| format!("item: no gear rig {}", g.rig))?;
        let n = hull.root.walk().into_iter().find(|n| n.name == g.node).ok_or_else(|| format!("item: no hull node {}", g.node))?;
        if !model_bones(n.model.as_deref().unwrap_or(""))?.contains(&g.bone.to_lowercase()) {
            return Err(format!("item: no bone {} on {}", g.bone, g.node));
        }
    }
    for m in &ship.machines {
        for s in &m.states {
            for a in &s.activities {
                let at = |what: String| format!("machine {} of {}, state {}: {what}", m.name, m.file, s.name);
                let Activity::Known(k) = a else { continue };
                let driven: Vec<&str> = match k {
                    KnownActivity::SetKinematicValueMultiple { values } => values.keys().map(String::as_str).collect(),
                    KnownActivity::KinematicBezierCurveMultiple(c) => c.curves.iter().filter_map(|c| c.value.as_deref()).collect(),
                    KnownActivity::PointLightActivity(l) | KnownActivity::SpotLightActivity(l) => {
                        for b in &l.bones {
                            let (model, bone) = b.split_once('!').ok_or_else(|| at(format!("light bone {b} is not Model!Bone")))?;
                            if !model_bones(model).map_err(&at)?.contains(&format!("!{bone}").to_lowercase()) {
                                return Err(at(format!("no light bone {b}")));
                            }
                        }
                        vec![]
                    }
                    _ => vec![],
                };
                if let Some(v) = driven.iter().find(|v| !motions.contains(*v)) {
                    return Err(at(format!("{v} is no rig's motion")));
                }
            }
        }
    }
    ctx.read(&ship.paint.texture_file)?;
    Ok(())
}

/// The ship's rest on its gear, from the real models posed by the rigs (for the task that replaces the recreated gear's
/// numbers): the feet's lowest point and each foot's contact (the mean of its vertices within 1 cm of its lowest) in the
/// hull's model space, gear Down with the hull's extension Low, and gear ShipRaised with it High.
#[test]
#[ignore]
fn gear_rest_heights() {
    use glam::{DMat4, DVec3};
    use se_core::elite_rig::{attach_base, elite_matrix, pose, state_pose, BoneTable};
    let dir = super::out_dir();
    let ship = parse_ship(&std::fs::read(dir.join("ship.json")).unwrap()).unwrap();
    let rigs: Vec<&Rig> = ship.rigs.iter().filter(|r| r.file == "hull_sidewinder" || r.file.starts_with("gear_")).collect();
    let mut edms = BTreeMap::new();
    let mut bones = BoneTable::default();
    for n in rigs.iter().flat_map(|r| r.root.walk()) {
        let Some(m) = &n.model else { continue };
        if edms.contains_key(m) {
            continue;
        }
        let edm = parse_edm(&std::fs::read(dir.join(format!("models/{m}.edm"))).unwrap()).unwrap();
        for p in &edm.parts {
            let inv = p.inverse_bind.unwrap().map(f64::from);
            bones.insert(m, &p.name, elite_matrix(&inv).inverse());
        }
        edms.insert(m.clone(), edm);
    }
    let hull = rigs[0];
    let ext = ship.machine("hull_sidewinder", "LandingGearExtension").unwrap();
    for (stance, hull_state) in [("Down", "Low"), ("ShipRaised", "High")] {
        let hull_poses = pose(&hull.root, &bones, &state_pose(ext.state(hull_state).unwrap()), DMat4::IDENTITY).unwrap();
        let to_hull = hull_poses[0].world.inverse();
        let mut lowest = f64::MAX;
        for g in &ship.item.gear {
            let rig = rigs.iter().find(|r| r.name == g.rig).unwrap();
            let values = state_pose(ship.machine(&rig.file, "LandingGearCSM").unwrap().state(stance).unwrap());
            let base = attach_base(&hull_poses, &bones, &g.node, &g.bone).unwrap();
            let mut pts = Vec::new();
            for p in pose(&rig.root, &bones, &values, base).unwrap() {
                let Some(edm) = p.node.model.as_ref().map(|m| &edms[m]) else { continue };
                for v in edm.draws.iter().flat_map(|d| &d.vertices) {
                    let q = v.decode().position.map(f64::from);
                    pts.push(to_hull.transform_point3(p.world.transform_point3(DVec3::from(q))));
                }
            }
            let min = pts.iter().map(|p| p.y).fold(f64::MAX, f64::min);
            let foot: Vec<&DVec3> = pts.iter().filter(|p| p.y < min + 0.01).collect();
            let contact = foot.iter().copied().sum::<DVec3>() / foot.len() as f64;
            println!("{stance}: {} lowest y {min:.4} m, contact ({:.4}, {:.4}, {:.4}) from {} vertices", g.rig, contact.x, contact.y, contact.z, foot.len());
            lowest = lowest.min(min);
        }
        let hull_y = hull_poses[0].world.w_axis.y;
        println!("{stance} (hull extension {hull_state}): rest height {:.4} m (the hull's origin over the feet; the hull node at y {hull_y:.4} in the ship's frame)", -lowest);
    }
}
