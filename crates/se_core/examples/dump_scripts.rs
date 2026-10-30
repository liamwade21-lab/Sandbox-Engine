//! Parse every shipped `.se` script and report warnings and unmapped commands.
//! With an output dir argument also writes the raw scripts there.
use se_core::se_script::{parse, Op};
use se_core::vfs::Vfs;
use std::collections::BTreeMap;
use std::path::PathBuf;
fn main() {
    let out = std::env::args().nth(1).map(PathBuf::from);
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let mut other: BTreeMap<String, usize> = BTreeMap::new();
    for dir in ["scripts", "scripts/menu", "scripts/tutorials"] {
        for f in vfs.list_ext(dir, "se") {
            let s = vfs.read_string(&f).unwrap();
            if let Some(o) = &out {
                std::fs::write(o.join(f.replace('/', "__")), &s).unwrap();
            }
            let sc = parse(&s);
            println!("{f}: {} commands, {} warnings", sc.commands.len(), sc.warnings.len());
            for w in &sc.warnings {
                println!("  warn: {w}");
            }
            for c in &sc.commands {
                if let Op::Other(i) = &c.op {
                    *other.entry(i.key.to_ascii_lowercase()).or_default() += 1;
                }
            }
        }
    }
    println!("untyped commands: {other:?}");
}
