//! Survey the shipped `.se` scripts (in memory, nothing is written): variables touched by
//! Set/SetForce/Interpolate/WaitVar, trigger kinds with their parameter keys, and the
//! arguments of the GUI commands. Usage: script_survey [vars|triggers|gui|lines FILE A B]
use se_core::se_script::{parse, Op};
use se_core::vfs::Vfs;
use std::collections::{BTreeMap, BTreeSet};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let what = a.first().map(String::as_str).unwrap_or("vars");
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    if what == "lines" {
        let t = vfs.read_string(&a[1]).unwrap();
        let (from, to): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
        for (i, l) in t.lines().enumerate().skip(from - 1).take(to + 1 - from) {
            println!("{}: {l}", i + 1);
        }
        return;
    }
    if what == "brief" {
        // Non-blank lines, comments dropped, long strings cut to 50 characters.
        let t = vfs.read_string(&a[1]).unwrap();
        for (i, l) in t.lines().enumerate() {
            let l = l.split("//").next().unwrap_or("").trim_end();
            if l.trim().is_empty() {
                continue;
            }
            let cut: String = if l.chars().count() > 90 { l.chars().take(90).collect::<String>() + "..." } else { l.to_string() };
            println!("{}:{}", i + 1, cut.replace('\t', " "));
        }
        return;
    }
    if what == "music" {
        // The tracks the scripts' PlayMusic names, as the music player finds them.
        let all = vfs.list_ext("music", "ogg");
        for name in a.iter().skip(1) {
            let hit = all.iter().find(|p| p.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(name)));
            println!("{name}: {hit:?} (direct exists: {})", vfs.exists(&format!("music/{name}")));
        }
        return;
    }
    if what == "sizes" {
        for dir in ["scripts", "scripts/menu", "scripts/tutorials"] {
            for f in vfs.list_ext(dir, "se") {
                let t = vfs.read_string(&f).unwrap();
                println!("{f}: {} lines, {} bytes", t.lines().count(), t.len());
            }
        }
        return;
    }
    let mut vars: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut trig: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    let mut gui: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for dir in ["scripts", "scripts/menu", "scripts/tutorials"] {
        for f in vfs.list_ext(dir, "se") {
            let s = vfs.read_string(&f).unwrap();
            let short = f.rsplit('/').next().unwrap().to_string();
            for c in parse(&s).commands {
                match &c.op {
                    Op::Set { name, value, force } => {
                        let k = if *force { "SetForce" } else { "Set" };
                        vars.entry(name.clone()).or_default().insert(format!("{k}={value:?}"));
                    }
                    Op::Interpolate { name, to, .. } => {
                        vars.entry(name.clone()).or_default().insert(format!("Interp={to}"));
                    }
                    Op::WaitVar { name, wait } => {
                        vars.entry(name.clone()).or_default().insert(format!("WaitVar={wait:?}"));
                    }
                    Op::WaitTrigger(t) => {
                        let e = trig.entry(t.kind.clone()).or_default();
                        e.0 += 1;
                        for r in &t.raw.items {
                            e.1.insert(format!("{} {:?}", r.key, r.args));
                        }
                    }
                    Op::Other(i) => {
                        let arg = i.args.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(" ");
                        let blk = i
                            .block()
                            .map(|b| b.items.iter().map(|r| r.key.clone()).collect::<Vec<_>>().join(","))
                            .unwrap_or_default();
                        let _ = &short;
                        *gui.entry(i.key.to_ascii_lowercase()).or_default().entry(format!("{arg} {{{blk}}}")).or_default() += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    match what {
        "vars" => {
            for (k, v) in &vars {
                println!("{k}: {}", v.iter().cloned().collect::<Vec<_>>().join(" | "));
            }
        }
        "triggers" => {
            for (k, (n, p)) in &trig {
                println!("{k} x{n}: {}", p.iter().cloned().collect::<Vec<_>>().join("; "));
            }
        }
        _ => {
            for (k, m) in &gui {
                println!("{k}:");
                for (a, n) in m {
                    println!("  {n}x {a}");
                }
            }
        }
    }
}
