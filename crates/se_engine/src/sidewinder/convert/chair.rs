//! The pilot chair's animation (T3c): `chair/clips.json` + `chair/clips.bin` and `chair/graph.json`
//! (`se_core::elite_ship`'s chair shapes; the rules that play them are `se_core::elite_chair`), from the decoded export's
//! `animation/chair_clips.json`, `chair_clips.bin` (the game's own decoder's output) and `chair_motiongraph.json`. Values
//! are Elite's (grade A); each shipped clip's keys are the export's bytes unchanged.
//!
//! What ships: the states a Sidewinder's seated solo pilot's chair can be in. The chair template writes `Mode_Ship` when
//! he sits down and `Empty_Ship` when he gets up, and the graph starts at `Uninitialised`
//! (`re/elite/notes/agent_motiongraph.md` 2.4, 2.6); the root handler picks only those three, so the states are theirs
//! plus every state on the cheapest routes between them, with the activities those states start and the clips those
//! name. The crew roles' `Action_*` modes (fighter and gunner seats), the buggy's modes and their clips are never
//! selected for a solo pilot and are left out; so is `chair_activatefast`, which no state plays.
//!
//! Each export record is rewritten as JSON in the mod's shape (its fields picked, a few renamed, no value changed) and
//! read into `se_core`'s type, which refuses any field it doesn't know. `build` works the files out from the export
//! alone; `convert` writes them and reads them back, `verify` compares the folder's with a fresh build without writing
//! (`shipped_files_are_the_exports`); `check` tests them against the export's bytes, the shipped chair model and
//! `Chair::new`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use se_core::elite_asset::parse_edm;
use se_core::elite_chair::{Chair, ChairSkeleton};
use se_core::elite_ship::{
    parse_chair_clips, parse_chair_graph, write_chair_clips, write_chair_graph, ChairActivity, ChairBlock, ChairClip, ChairClips, ChairGraph, ChairHierarchy,
    ChairOutput, ChairState, ChairTemplates, ChairTransition, ChairVariable, CHAIR_FORMAT,
};

use super::models::read_json;
use super::{Ctx, Shipped};

const CLIPS_JSON: &str = "chair/clips.json";
const CLIPS_BIN: &str = "chair/clips.bin";
const GRAPH_JSON: &str = "chair/graph.json";
const CHAIR_MODEL: &str = "models/cockpit_chair.edm";
const EXPORT_CLIPS: &str = "animation/chair_clips.json";
const EXPORT_KEYS: &str = "animation/chair_clips.bin";
const EXPORT_GRAPH: &str = "animation/chair_motiongraph.json";
/// A block's path cost that rules it out of routing.
const NO_ROUTE: u64 = 0xffff;

// ------------------------------------------------------------------------------------------------ reading the export

fn get<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a Value, String> {
    v.get(key).ok_or_else(|| format!("{at}: no {key}"))
}

fn list<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a [Value], String> {
    get(v, key, at)?.as_array().map(Vec::as_slice).ok_or_else(|| format!("{at}: {key} is not a list"))
}

fn text<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a str, String> {
    get(v, key, at)?.as_str().ok_or_else(|| format!("{at}: {key} is not text"))
}

fn whole(v: &Value, key: &str, at: &str) -> Result<u64, String> {
    get(v, key, at)?.as_u64().ok_or_else(|| format!("{at}: {key} is not a whole number"))
}

/// An object with only `keys` of `v` (missing ones null): an export record cut to the mod's fields.
fn pick(v: &Value, keys: &[&str]) -> Value {
    Value::Object(keys.iter().map(|k| (k.to_string(), v.get(*k).cloned().unwrap_or(Value::Null))).collect::<Map<_, _>>())
}

fn typed<T>(v: Value, read: fn(Value) -> Result<T, serde_json::Error>, at: &str) -> Result<T, String> {
    read(v).map_err(|e| format!("{at}: {e}"))
}

const KEY_FIELDS: &[&str] = &["x", "y", "interp", "a", "b"];

fn keys(v: Option<&Value>) -> Value {
    Value::Array(v.and_then(Value::as_array).map(|k| k.iter().map(|k| pick(k, KEY_FIELDS)).collect()).unwrap_or_default())
}

/// A ParamBlock in the mod's shape (`ChairParam`).
fn param(p: &Value) -> Value {
    json!({ "const": p.get("const"), "bound": p.get("bound"), "variable": p.get("variable"), "keys": keys(p.get("keys")) })
}

/// An activity in the mod's shape. Fields the shape drops must hold nothing: the per-activity transitions merged in at a
/// state change (count 0) and an AnimationActivity's data streams (`stream_count` 0); anything else is refused.
fn activity(id: &str, a: &Value) -> Result<ChairActivity, String> {
    let at = format!("{EXPORT_GRAPH}: activity {id}");
    if whole(get(a, "per_activity_transitions", &at)?, "count", &at)? != 0 {
        return Err(format!("{at}: per-activity transitions, which the chair's rules don't run"));
    }
    let v = match text(a, "type", &at)? {
        "AnimationActivity" => {
            if whole(a, "stream_count", &at)? != 0 {
                return Err(format!("{at}: data streams, which the chair's rules don't run"));
            }
            let mut v = pick(a, &["type", "clip", "flags", "start_phase", "phase_variable", "phase_offset", "phase_output_variable"]);
            v["layer"] = get(a, "layer_arg", &at)?.clone();
            v["weight"] = param(get(a, "weight", &at)?);
            v["rate"] = param(get(a, "rate", &at)?);
            v
        }
        "GroupedActivityActivity" => pick(a, &["type", "children"]),
        "TriggeredActivityActivity" => pick(a, &["type", "trigger_variable", "activity", "fade_in_s", "fade_out_s", "hold_min_flag", "min_time_s", "output_variable"]),
        "JoystickButtonActivity" => pick(a, &["type", "button_variable", "in", "loop", "out"]),
        other => return Err(format!("{at}: type {other}, which the chair's rules don't know")),
    };
    typed(v, serde_json::from_value, &at)
}

/// A transition block's first group (the only one the exe reads).
fn group0<'a>(blocks: &'a Value, id: &str) -> Result<&'a Value, String> {
    let at = format!("{EXPORT_GRAPH}: block {id}");
    list(get(blocks, id, EXPORT_GRAPH)?, "groups", &at)?.first().ok_or_else(|| format!("{at}: no group"))
}

fn block(blocks: &Value, id: &str) -> Result<ChairBlock, String> {
    let g = group0(blocks, id)?;
    if ["route_via_state", "event_variable", "explicit_transitions_ptr", "decision_node"].iter().any(|k| g.get(*k).is_some_and(|v| !v.is_null())) {
        return Err(format!("{EXPORT_GRAPH}: block {id} routes through a state, waits for an event, lists transitions or asks a node: the chair's rules don't"));
    }
    let markers = text(g, "marker_slots_hex", EXPORT_GRAPH)?;
    if whole(g, "u16_0x28", EXPORT_GRAPH)? != 0 || markers.chars().any(|c| c != '0') {
        return Err(format!("{EXPORT_GRAPH}: block {id} lists per-activity transitions or waits for an activity marker: the chair's rules don't"));
    }
    let mut v = pick(g, &["at", "fade_time_s", "wait_flag", "wait_slack_s", "route_by_path_table", "path_cost"]);
    v["curve"] = if g.get("curve").is_some_and(|c| !c.is_null()) { keys(g.get("curve")) } else { Value::Null };
    typed(v, serde_json::from_value, &format!("{EXPORT_GRAPH}: block {id}"))
}

/// A template field's value (`chair_templates.COMPONENT.fields[off]`).
fn field<'a>(tpl: &'a Value, component: &str, off: &str) -> Result<&'a Value, String> {
    tpl.get(component)
        .and_then(|c| c.get("fields"))
        .and_then(Value::as_array)
        .and_then(|f| f.iter().find(|f| f.get("off").and_then(Value::as_str) == Some(off)))
        .and_then(|f| f.get("value"))
        .ok_or_else(|| format!("{EXPORT_GRAPH}: no template field {component} +{off}"))
}

/// The template values a seated pilot needs (`ChairTemplates`; the offsets are the notes').
fn templates(tpl: &Value) -> Result<ChairTemplates, String> {
    let (cm, cc, ck) = ("ChairMotiongraphComponent", "CockpitCollisionAnimationComponent", "CockpitComponent");
    let mut table: Vec<(u64, Value)> = field(tpl, cc, "0x10")?
        .as_array()
        .ok_or_else(|| format!("{EXPORT_GRAPH}: {cc} +0x10 is not the event table"))?
        .iter()
        .map(|e| Some((e.get("index")?.as_u64()?, e.get("name")?.clone())))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("{EXPORT_GRAPH}: {cc} +0x10: an entry without index and name"))?;
    table.sort_by_key(|(i, _)| *i);
    let v = json!({
        "loop_handler": field(tpl, cm, "0x78")?,
        "initial_loop": field(tpl, "MotionGraphComponent", "0x58")?,
        "occupied_loop": field(tpl, cm, "0x80")?,
        "vacated_loop": field(tpl, cm, "0x90")?,
        "throttle_variable": field(tpl, cm, "0x10")?,
        "throttle_control": field(tpl, cm, "0x18")?,
        "trigger_variable": field(tpl, cm, "0x20")?,
        "trigger_control": field(tpl, cm, "0x28")?,
        "pitch_control": field(tpl, cm, "0x30")?,
        "roll_control": field(tpl, cm, "0x38")?,
        "yaw_control": field(tpl, cm, "0x40")?,
        "joystick_weight_variable": field(tpl, cm, "0x48")?,
        "joystick_prop_variable": field(tpl, cm, "0x50")?,
        "joystick_yaw_variable": field(tpl, cm, "0x58")?,
        "buffet_events": table.into_iter().map(|(_, n)| n).collect::<Vec<_>>(),
        "buffet_intensity_variable": field(tpl, cc, "0x20")?,
        "buffet_speed_variable": field(tpl, cc, "0x28")?,
        "buffet_thresholds": [field(tpl, ck, "0x40")?, field(tpl, ck, "0x44")?, field(tpl, ck, "0x48")?],
    });
    typed(v, serde_json::from_value, &format!("{EXPORT_GRAPH}: chair_templates"))
}

/// A state's transitions as (block, targets).
fn transitions(s: &Value, at: &str) -> Result<Vec<(String, Vec<u32>)>, String> {
    list(s, "transitions", at)?
        .iter()
        .map(|t| {
            let targets = list(t, "target_states", at)?.iter().map(|n| n.as_u64().map(|n| n as u32).ok_or_else(|| format!("{at}: a target is not a state id"))).collect::<Result<_, _>>()?;
            Ok((text(t, "block", at)?.to_string(), targets))
        })
        .collect()
}

/// The cheapest route from `a` to `b` over the listed transitions (Dijkstra on the blocks' path costs), `a` first; just
/// `[a]` when none leads there.
fn route(edges: &HashMap<u32, Vec<(u64, Vec<u32>)>>, a: u32, b: u32) -> Vec<u32> {
    let mut dist: HashMap<u32, u64> = HashMap::from([(a, 0)]);
    let mut prev: HashMap<u32, u32> = HashMap::new();
    let mut open: BTreeSet<(u64, u32)> = BTreeSet::from([(0, a)]);
    while let Some((d, s)) = open.pop_first() {
        for (cost, targets) in edges.get(&s).into_iter().flatten() {
            if *cost == NO_ROUTE {
                continue;
            }
            for &n in targets {
                if dist.get(&n).map_or(true, |&o| d + cost < o) {
                    dist.insert(n, d + cost);
                    prev.insert(n, s);
                    open.insert((d + cost, n));
                }
            }
        }
    }
    let mut path = vec![b];
    while path[path.len() - 1] != a {
        match prev.get(&path[path.len() - 1]) {
            Some(&p) => path.push(p),
            None => return vec![a],
        }
    }
    path.reverse();
    path
}

/// `chair/graph.json` from the export, and how many states it leaves out.
fn build_graph(mg: &Value) -> Result<(ChairGraph, usize), String> {
    let f = EXPORT_GRAPH;
    let g = get(mg, "graph", f)?;
    let blocks = get(g, "transition_blocks", f)?;
    let t = templates(get(mg, "chair_templates", f)?)?;
    let outputs = list(g, "root_outputs", f)?;
    let output = |value: &str| {
        outputs
            .iter()
            .find(|o| o.get("kind").and_then(Value::as_str) == Some("state") && o.get("enum_name").and_then(Value::as_str) == Some(value))
            .and_then(|o| o.get("state_id")?.as_u64())
            .map(|s| s as u32)
            .ok_or_else(|| format!("{f}: the loop value {value} selects no state"))
    };
    let mut all: BTreeMap<u32, (&Value, Vec<(String, Vec<u32>)>)> = BTreeMap::new();
    for s in list(g, "states", f)? {
        let id = whole(s, "id", f)? as u32;
        all.insert(id, (s, transitions(s, &format!("{f}: state {id}"))?));
    }
    let mut edges: HashMap<u32, Vec<(u64, Vec<u32>)>> = HashMap::new();
    for (id, (_, ts)) in &all {
        for (b, targets) in ts {
            edges.entry(*id).or_default().push((whole(group0(blocks, b)?, "path_cost", f)?, targets.clone()));
        }
    }
    let ends = [output(&t.initial_loop)?, output(&t.occupied_loop)?, output(&t.vacated_loop)?];
    let mut keep: BTreeSet<u32> = ends.into_iter().collect();
    for a in ends {
        for b in ends {
            keep.extend(route(&edges, a, b));
        }
    }
    let mut states = Vec::new();
    for &id in &keep {
        let (s, ts) = all.get(&id).ok_or_else(|| format!("{f}: no state {id}"))?;
        let activities = list(s, "activities", f)?.iter().map(|a| a.as_str().map(str::to_string).ok_or_else(|| format!("{f}: state {id}: an activity id is not text"))).collect::<Result<_, _>>()?;
        let transitions = ts
            .iter()
            .filter_map(|(block, targets)| {
                let targets: Vec<u32> = targets.iter().copied().filter(|n| keep.contains(n)).collect();
                (!targets.is_empty()).then(|| ChairTransition { block: block.clone(), targets })
            })
            .collect();
        states.push(ChairState { id, activities, transitions });
    }
    let exported = get(g, "activities", f)?;
    let mut activities = BTreeMap::new();
    let mut todo: Vec<String> = states.iter().flat_map(|s| s.activities.iter().cloned()).collect();
    while let Some(id) = todo.pop() {
        if !activities.contains_key(&id) {
            let a = activity(&id, get(exported, &id, f)?)?;
            todo.extend(a.children().into_iter().cloned());
            activities.insert(id, a);
        }
    }
    let root = get(g, "root", f)?;
    let default_block = text(root, "default_transition_block", f)?.to_string();
    let mut transition_blocks = BTreeMap::new();
    for id in states.iter().flat_map(|s| s.transitions.iter().map(|t| t.block.clone())).chain([default_block.clone()]) {
        let b = block(blocks, &id)?;
        transition_blocks.insert(id, b);
    }
    let decision = get(get(g, "decision_nodes", f)?, text(root, "decision_root", f)?, f)?;
    let action_pattern = text(get(decision, "params", f)?, "VariableNamePattern", f)?.to_string();
    let variables: Vec<ChairVariable> = list(mg, "variables", f)?
        .iter()
        .map(|v| typed(pick(v, &["name", "type", "type_name", "min", "max", "default_i32"]), serde_json::from_value, &format!("{f}: a variable")))
        .collect::<Result<_, _>>()?;
    let enums = get(mg, "enums", f)?;
    let names = |key: &str| -> Result<Vec<String>, String> { typed(get(enums, key, f)?.clone(), serde_json::from_value, &format!("{f}: {key}")) };
    let root_outputs = outputs
        .iter()
        .filter_map(|o| {
            let state = o.get("state_id")?.as_u64()? as u32;
            if o.get("kind")?.as_str()? != "state" || !keep.contains(&state) {
                return None;
            }
            Some(ChairOutput { key: o.get("key")?.as_u64()? as u32, enum_name: o.get("enum_name")?.as_str()?.to_string(), state })
        })
        .collect();
    let h = get(mg, "hier", f)?;
    let hierarchy: ChairHierarchy = typed(
        json!({
            "flags": h.get("flags"),
            "top_node": h.get("top_node_index"),
            "nodes": list(h, "nodes", f)?.iter().map(|n| pick(n, &["name", "type", "type_name"])).collect::<Vec<_>>(),
            "links": list(h, "links", f)?.iter().map(|l| json!([l.get("parent"), l.get("child")])).collect::<Vec<_>>(),
        }),
        serde_json::from_value,
        &format!("{f}: hier"),
    )?;
    let source = get(mg, "source", f)?;
    let chair = ChairGraph {
        format: CHAIR_FORMAT,
        source: text(source, "file", f)?.to_string(),
        build: text(source, "build", f)?.to_string(),
        variables,
        loop_enum: names("cockpitchairaction_loopenum")?,
        oneshot_enum: names("cockpitchairaction_oneshotenum")?,
        action_pattern,
        root_outputs,
        states,
        activities,
        transition_blocks,
        default_block,
        hierarchy,
        templates: t,
    };
    // the file's own checks
    parse_chair_graph(&write_chair_graph(&chair)).map_err(|e| e.to_string())?;
    Ok((chair, all.len() - keep.len()))
}

/// The byte range of a clip's rotation or position keys in its key file.
fn range(at: u64, frames: u64, channels: usize) -> std::ops::Range<usize> {
    at as usize..(at + 8 * frames * channels as u64) as usize
}

/// An export clip's frames and its (rotation, position) key offsets and channel counts.
fn key_layout(c: &Value, at: &str) -> Result<(u64, [(u64, usize); 2]), String> {
    Ok((
        whole(c, "frames", at)?,
        [(whole(c, "rot_keys_at", at)?, list(c, "rot_channels", at)?.len()), (whole(c, "pos_keys_at", at)?, list(c, "pos_channels", at)?.len())],
    ))
}

/// Every export clip with its set's name.
fn export_clips(meta: &Value) -> Result<Vec<(&str, &Value)>, String> {
    let mut out = Vec::new();
    for set in list(meta, "sets", EXPORT_CLIPS)? {
        let name = text(set, "name", EXPORT_CLIPS)?;
        out.extend(list(set, "clips", EXPORT_CLIPS)?.iter().map(|c| (name, c)));
    }
    Ok(out)
}

/// `chair/clips.json` + `.bin` for the clips the graph names (the export's order), each clip's keys copied.
fn build_clips(meta: &Value, blob: &[u8], wanted: &BTreeSet<String>) -> Result<ChairClips, String> {
    let mut clips = Vec::new();
    let mut keys = Vec::new();
    for (set, c) in export_clips(meta)? {
        let name = text(c, "name", EXPORT_CLIPS)?;
        if !wanted.contains(&name.to_lowercase()) {
            continue;
        }
        let at = format!("{EXPORT_CLIPS}: clip {name}");
        let (frames, layout) = key_layout(c, &at)?;
        let mut offsets = [0u64; 2];
        for (o, (from, channels)) in offsets.iter_mut().zip(layout) {
            *o = keys.len() as u64;
            keys.extend_from_slice(blob.get(range(from, frames, channels)).ok_or_else(|| format!("{EXPORT_KEYS}: clip {name}'s keys run past its end"))?);
        }
        let mut v = pick(c, &["name", "duration_s", "frames", "flags", "additive", "fps", "skeleton_bones", "pos_offset", "pos_scale"]);
        v["set"] = Value::from(set);
        v["rot_channels"] = list(c, "rot_channels", &at)?.iter().map(|ch| pick(ch, &["bone", "skeleton_index", "in_chair"])).collect();
        v["pos_channels"] = list(c, "pos_channels", &at)?.iter().map(|ch| pick(ch, &["bone", "skeleton_index", "in_chair", "table"])).collect();
        v["rot_keys_at"] = offsets[0].into();
        v["pos_keys_at"] = offsets[1].into();
        clips.push(typed::<ChairClip>(v, serde_json::from_value, &at)?);
    }
    let found: BTreeSet<String> = clips.iter().map(|c| c.name.to_lowercase()).collect();
    if let Some(n) = wanted.difference(&found).next() {
        return Err(format!("{EXPORT_CLIPS}: no clip {n}, which the graph names"));
    }
    let doc = ChairClips {
        format: CHAIR_FORMAT,
        source: text(meta, "source", EXPORT_CLIPS)?.to_string(),
        skeleton: CHAIR_MODEL.into(),
        keys_file: CLIPS_BIN.into(),
        key_layout: text(meta, "key_layout", EXPORT_CLIPS)?.to_string(),
        clips,
        keys: Vec::new(),
    };
    // the keys as the parser reads them, with the file's own checks
    parse_chair_clips(&write_chair_clips(&doc).0, &keys).map_err(|e| e.to_string())
}

/// Everything `chair/` holds, from the export alone.
struct Built {
    clips: ChairClips,
    graph: ChairGraph,
    export: Value,
    export_keys: Vec<u8>,
    states_left_out: usize,
    sources: Vec<String>,
}

fn build(ctx: &Ctx) -> Result<Built, String> {
    let export = read_json(&ctx.export.join(EXPORT_CLIPS))?;
    let export_keys = std::fs::read(ctx.export.join(EXPORT_KEYS)).map_err(|e| format!("{EXPORT_KEYS}: {e}"))?;
    let mg = read_json(&ctx.export.join(EXPORT_GRAPH))?;
    let (graph, states_left_out) = build_graph(&mg)?;
    let wanted: BTreeSet<String> = graph.clips().into_iter().map(str::to_lowercase).collect();
    let clips = build_clips(&export, &export_keys, &wanted)?;
    let sources = vec![
        format!("decoded/{EXPORT_CLIPS} and {EXPORT_KEYS} (Elite's {})", clips.source),
        format!("decoded/{EXPORT_GRAPH} (Elite's {}, build {})", graph.source, graph.build),
    ];
    Ok(Built { clips, graph, export, export_keys, states_left_out, sources })
}

/// The read-back's checks: each clip's keys are the export's bytes; every channel's bone is the chair model's by name
/// and index, but the bones the export says the chair lacks (and those must be absent); every clip the graph names is
/// shipped; `Chair::new` takes the three files with the shipped skeleton.
fn check(ctx: &Ctx, b: &Built, clips: &ChairClips, graph: &ChairGraph) -> Result<(), String> {
    let keys: Vec<u8> = clips.keys.iter().flat_map(|k| k.to_le_bytes()).collect();
    let export = export_clips(&b.export)?;
    for c in &clips.clips {
        let (_, e) = export.iter().find(|(_, e)| e.get("name").and_then(Value::as_str) == Some(c.name.as_str())).ok_or_else(|| format!("{CLIPS_JSON}: clip {} is not the export's", c.name))?;
        let (frames, layout) = key_layout(e, EXPORT_CLIPS)?;
        let ours = [(c.rot_keys_at, c.rot_channels.len()), (c.pos_keys_at, c.pos_channels.len())];
        for ((at, n), (from, m)) in ours.into_iter().zip(layout) {
            if n != m || keys.get(range(at, c.frames as u64, n)) != b.export_keys.get(range(from, frames, m)) {
                return Err(format!("{CLIPS_BIN}: clip {}'s keys differ from the export's", c.name));
            }
        }
    }
    let lacking: Vec<&str> = list(&b.export, "bones_without_chair_bone", EXPORT_CLIPS)?.iter().filter_map(Value::as_str).collect();
    let edm = parse_edm(&ctx.read(CHAIR_MODEL)?).map_err(|e| format!("{CHAIR_MODEL}: {e}"))?;
    for c in &clips.clips {
        for ch in c.rot_channels.iter().chain(&c.pos_channels) {
            let named = |n: &str| n.eq_ignore_ascii_case(&ch.bone);
            let ok = if ch.in_chair {
                edm.parts.get(ch.skeleton_index as usize).is_some_and(|p| named(&p.name)) && !lacking.iter().any(|n| named(n))
            } else {
                lacking.iter().any(|n| named(n)) && !edm.parts.iter().any(|p| named(&p.name))
            };
            if !ok {
                return Err(format!("{CLIPS_JSON}: clip {}: bone {} (index {}) is not {CHAIR_MODEL}'s as the clip says", c.name, ch.bone, ch.skeleton_index));
            }
        }
    }
    if let Some(n) = graph.clips().into_iter().find(|n| clips.clip(n).is_none()) {
        return Err(format!("{GRAPH_JSON}: clip {n} is not shipped"));
    }
    let skeleton = ChairSkeleton::from_edm(&edm).map_err(|e| format!("{CHAIR_MODEL}: {e}"))?;
    Chair::new(&skeleton, Arc::new(clips.clone()), Arc::new(graph.clone())).map(|_| ()).map_err(|e| e.to_string())
}

fn read_folder(ctx: &Ctx) -> Result<(ChairClips, ChairGraph), String> {
    let clips = parse_chair_clips(&ctx.read(CLIPS_JSON)?, &ctx.read(CLIPS_BIN)?).map_err(|e| e.to_string())?;
    let graph = parse_chair_graph(&ctx.read(GRAPH_JSON)?).map_err(|e| e.to_string())?;
    Ok((clips, graph))
}

/// The folder's `chair/` is what a fresh build makes, and passes `check`. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let b = build(ctx)?;
    let (json, bin) = write_chair_clips(&b.clips);
    if ctx.read(CLIPS_JSON)? != json || ctx.read(CLIPS_BIN)? != bin || ctx.read(GRAPH_JSON)? != write_chair_graph(&b.graph) {
        return Err("chair/ differs from what the decoded files give".into());
    }
    let (clips, graph) = read_folder(ctx)?;
    check(ctx, &b, &clips, &graph)?;
    println!("chair/: {} states, {} clips match the export", graph.states.len(), clips.clips.len());
    Ok(())
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let b = build(ctx)?;
    let (json, bin) = write_chair_clips(&b.clips);
    ctx.write(CLIPS_JSON, &json)?;
    ctx.write(CLIPS_BIN, &bin)?;
    ctx.write(GRAPH_JSON, &write_chair_graph(&b.graph))?;
    let (clips, graph) = read_folder(ctx)?;
    if clips != b.clips || graph != b.graph {
        return Err("chair/: what was read back differs from what was written".into());
    }
    check(ctx, &b, &clips, &graph)?;
    let all = export_clips(&b.export)?.len();
    shipped.add(
        CLIPS_JSON,
        &b.sources[..1],
        "A",
        "The pilot chair's clips that a seated solo pilot's chair plays: duration, frames, flags, channels (bone, chair skeleton \
         index, position offset and scale), where their keys sit in chair/clips.bin.",
    );
    shipped.add(CLIPS_BIN, &b.sources[..1], "A", "Those clips' 16-bit keys, the game's own decoder's output, each clip's bytes unchanged.");
    shipped.add(
        GRAPH_JSON,
        &b.sources,
        "A",
        "The chair's motion graph CockpitChairMotionGraph for the states a seated solo pilot's chair can be in (Uninitialised, \
         Empty_Ship, Mode_Ship and the routes between them): activities, transition blocks, variables, enums, the blend \
         hierarchy, the chair templates' values. Elite's values.",
    );
    shipped.note(&format!(
        "Chair: {} of the motion graph's {} states and {} of the {all} chair clips are shipped. Left out: the crew roles' \
         Action_* modes (fighter and gunner seats, their idles and transitions), the buggy's modes, and chair_activatefast, \
         which no state plays: a solo pilot's chair never reaches them (re/elite/notes/agent_motiongraph.md 2.4, 2.6).",
        graph.states.len(),
        graph.states.len() + b.states_left_out,
        clips.clips.len(),
    ));
    println!("chair/: {} states, {} clips, {} key bytes", graph.states.len(), clips.clips.len(), bin.len());
    Ok(())
}
