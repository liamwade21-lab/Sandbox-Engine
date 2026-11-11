//! The first-run ownership check (the user, 2026-11-07: friends may play it only if they own
//! SpaceEngine, whose files it reads). The first time it starts, the port asks for SpaceEngine.exe and
//! checks that Steam installed it for an account that has signed in on this computer:
//!
//! - the file is a Windows program whose version resource says SpaceEngine, by Cosmographic Software;
//! - it sits at `<library>\steamapps\common\<installdir>\system\SpaceEngine.exe`, beside `data`;
//! - `<library>\steamapps\appmanifest_314650.acf` (Steam's record of the installed app) names that
//!   folder and says the game is fully installed;
//! - Steam's own list of libraries (`<Steam>\steamapps\libraryfolders.vdf`) has that library, with
//!   SpaceEngine in it;
//! - the account that owns the install (the manifest's `LastOwner`) is one of the accounts in
//!   `<Steam>\config\loginusers.vdf`.
//!
//! The chosen exe is kept in `ownership.cfg` in the user folder, and every start checks it again
//! (a few small files: milliseconds), so it asks again only when the install has moved or gone.
//! Scripted runs (tests) never open a dialog: they check the copy Steam lists. Whatever path the port
//! reads SpaceEngine from (`--se-path`, `SE_PATH`) must pass the same check. Nothing is sent anywhere.
//!
//! It is a check for honest players, not copy protection: anyone who edits files can get past it.

use std::path::{Path, PathBuf};

use crate::config::EngineConfig;

/// SpaceEngine's Steam app id.
const APP_ID: &str = "314650";
const EXE: &str = "SpaceEngine.exe";
/// The window title of its dialogs.
const TITLE: &str = "Sandbox Engine";
/// Where the verified exe is kept (in the user folder).
const RECORD: &str = "ownership.cfg";

/// A SpaceEngine that passed the check.
#[derive(Debug, Clone, PartialEq)]
pub struct Verified {
    /// The install's top folder (holds `data`, `system`), what the port reads.
    pub root: PathBuf,
    pub exe: PathBuf,
    /// Its version resource's ProductVersion.
    pub version: String,
}

enum Stop {
    /// The player closed the dialog: quit without a message.
    Quit,
    /// Why it failed, shown to the player.
    Failed(String),
}

/// Runs the check before anything reads SpaceEngine's files, points `cfg.se_path` at the verified
/// install and returns a line for the log; quits the program when it fails or the player cancels.
pub fn check(cfg: &mut EngineConfig) -> String {
    match gate(cfg) {
        Ok((v, how)) => {
            cfg.se_path = v.root.clone();
            format!("SpaceEngine {} owned on Steam ({how}): {}", v.version, v.exe.display())
        }
        Err(Stop::Quit) => std::process::exit(0),
        Err(Stop::Failed(why)) => fail(cfg, &format!("Sandbox Engine needs SpaceEngine, installed through Steam.\n\n{why}")),
    }
}

/// Stops before the window opens with `text` in a message box (on stderr only in test runs).
pub fn fail(cfg: &EngineConfig, text: &str) -> ! {
    eprintln!("{text}");
    #[cfg(windows)]
    if !crate::settings::scripted(cfg) {
        win::message(text, win::ERROR);
    }
    let _ = cfg;
    std::process::exit(1)
}

/// Stops with a message when the port's own `assets` folder (shaders, textures, models) isn't
/// beside the exe: without it the menus draw over a black universe.
pub fn check_assets(cfg: &EngineConfig) {
    if crate::config::assets_found() {
        return;
    }
    let dir = crate::embedded_assets::target_dir().unwrap_or_else(crate::config::asset_dir);
    fail(
        cfg,
        &format!(
            "Sandbox Engine couldn't unpack its built-in files (shaders, textures, models) to:\n{}\n\n\
             Check that the disk has free space (about 150 MB) and that the folder can be written, then start it again.",
            dir.display()
        ),
    )
}

fn gate(cfg: &EngineConfig) -> Result<(Verified, &'static str), Stop> {
    let dirs = steam_dirs();
    if cfg.se_path_given {
        let exe = cfg.se_path.join("system").join(EXE);
        return verify(&exe, &dirs).map(|v| (v, "the path given")).map_err(Stop::Failed);
    }
    if let Some(exe) = read_record()
        && let Ok(v) = verify(&exe, &dirs)
    {
        return Ok((v, "checked before"));
    }
    let found = find_on_steam(&dirs);
    if crate::settings::scripted(cfg) || !cfg!(windows) {
        let exe = found.ok_or_else(|| Stop::Failed(format!("Steam doesn't list SpaceEngine (app {APP_ID}) in any of its libraries on this computer.")))?;
        return verify(&exe, &dirs).map(|v| (v, "found through Steam")).map_err(Stop::Failed);
    }
    // The copy Steam lists, when it passes, is kept without asking; the dialog only when there is none.
    if let Some(exe) = found.as_deref()
        && let Ok(v) = verify(exe, &dirs)
    {
        write_record(&v.exe);
        return Ok((v, "found through Steam"));
    }
    ask(&dirs, found.as_deref())
}

/// The first run: asks for the exe until one passes or the player cancels.
#[cfg(windows)]
fn ask(dirs: &[PathBuf], found: Option<&Path>) -> Result<(Verified, &'static str), Stop> {
    let mut intro = "Sandbox Engine runs on SpaceEngine's own files, so it needs your copy of SpaceEngine \
                     from Steam.\n\nClick OK, then select SpaceEngine.exe in your Steam library \
                     (steamapps\\common\\SpaceEngine\\system\\SpaceEngine.exe). It is asked only once."
        .to_string();
    if let Some(f) = found {
        intro.push_str(&format!("\n\nSteam has it here:\n{}", f.display()));
    }
    if !win::ok_cancel(&intro) {
        return Err(Stop::Quit);
    }
    let start = found.and_then(Path::parent);
    loop {
        let Some(exe) = win::pick_exe(start) else {
            return Err(Stop::Quit);
        };
        match verify(&exe, dirs) {
            Ok(v) => {
                write_record(&v.exe);
                return Ok((v, "selected now"));
            }
            Err(why) => {
                if !win::retry(&format!("That file can't be used:\n\n{why}\n\nRetry to pick another file, or Cancel to quit.")) {
                    return Err(Stop::Quit);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn ask(_: &[PathBuf], _: Option<&Path>) -> Result<(Verified, &'static str), Stop> {
    Err(Stop::Failed("Steam doesn't list SpaceEngine on this computer.".into()))
}

fn read_record() -> Option<PathBuf> {
    let text = std::fs::read_to_string(crate::settings::user_file(RECORD)).ok()?;
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    Some(PathBuf::from(line))
}

fn write_record(exe: &Path) {
    let path = crate::settings::user_file(RECORD);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{}\n", exe.display()));
}

/// Checks `exe` as the module's notes say; `steam` are Steam's own folders to look in (the
/// manifest's `LauncherPath` is tried too). `Err` says why in plain words.
pub fn verify(exe: &Path, steam: &[PathBuf]) -> Result<Verified, String> {
    let name_ok = exe.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(EXE));
    if !name_ok {
        return Err(format!("The file must be {EXE}."));
    }
    let bytes = std::fs::read(exe).map_err(|e| format!("{} can't be read ({e}).", exe.display()))?;
    if !windows_program(&bytes) {
        return Err(format!("{} isn't a Windows program.", exe.display()));
    }
    let company = version_string(&bytes, "CompanyName").unwrap_or_default();
    let product = version_string(&bytes, "ProductName").unwrap_or_default();
    let copyright = version_string(&bytes, "LegalCopyright").unwrap_or_default();
    if company != "SpaceEngine" || product != "SpaceEngine" || !copyright.contains("Cosmographic Software") {
        return Err(format!("{} isn't SpaceEngine (its details name \"{product}\" by \"{company}\").", exe.display()));
    }
    let version = version_string(&bytes, "ProductVersion").unwrap_or_default();

    // <library>\steamapps\common\<installdir>\system\SpaceEngine.exe
    fn named<'a>(p: Option<&'a Path>, want: &str) -> Option<&'a Path> {
        p.filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(want)))
    }
    let system = named(exe.parent(), "system");
    let root = system.and_then(Path::parent);
    let common = named(root.and_then(Path::parent), "common");
    let steamapps = named(common.and_then(Path::parent), "steamapps");
    let (Some(root), Some(steamapps)) = (root, steamapps) else {
        return Err(format!("{} isn't in a Steam library (steamapps\\common\\...\\system\\{EXE}).", exe.display()));
    };
    let library = steamapps.parent().unwrap_or(steamapps);
    if !root.join("data").is_dir() {
        return Err(format!("{} has no data folder beside it.", root.display()));
    }

    let manifest_path = steamapps.join(format!("appmanifest_{APP_ID}.acf"));
    let manifest = read_vdf(&manifest_path).ok_or_else(|| format!("Steam has no record of installing SpaceEngine in {} ({} is missing).", library.display(), manifest_path.display()))?;
    let app = manifest.get("AppState").ok_or("Steam's record of SpaceEngine can't be read.")?;
    let field = |k: &str| app.get(k).and_then(Vdf::str).unwrap_or("");
    let installdir = field("installdir");
    let folder = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if field("appid") != APP_ID || !installdir.eq_ignore_ascii_case(&folder) {
        return Err(format!("Steam's record says SpaceEngine is in \"{installdir}\", not \"{folder}\"."));
    }
    let flags: u32 = field("StateFlags").parse().unwrap_or(0);
    if flags & 4 == 0 {
        return Err("Steam says SpaceEngine isn't fully installed.".into());
    }
    let owner = field("LastOwner");

    let mut dirs: Vec<PathBuf> = steam.to_vec();
    if let Some(launcher) = Path::new(field("LauncherPath")).parent()
        && !launcher.as_os_str().is_empty()
    {
        dirs.push(launcher.to_path_buf());
    }
    let mut steam_root = None;
    for dir in &dirs {
        if let Some(list) = read_vdf(&dir.join("steamapps").join("libraryfolders.vdf"))
            && library_lists(&list, library)
        {
            steam_root = Some(dir.clone());
            break;
        }
    }
    let Some(steam_root) = steam_root else {
        return Err(format!("This computer's Steam doesn't list SpaceEngine in the library {}.", library.display()));
    };
    let users = read_vdf(&steam_root.join("config").join("loginusers.vdf"));
    let signed_in = users.as_ref().and_then(|u| u.get("users")).is_some_and(|u| !owner.is_empty() && owner != "0" && u.get(owner).is_some());
    if !signed_in {
        return Err("The Steam account that owns this SpaceEngine hasn't signed in to Steam on this computer.".into());
    }
    Ok(Verified { root: root.to_path_buf(), exe: exe.to_path_buf(), version })
}

/// Whether Steam's `libraryfolders.vdf` has `library` with SpaceEngine in it (an old-style list,
/// a bare path per entry, says nothing of its apps: the path alone counts then).
fn library_lists(list: &Vdf, library: &Path) -> bool {
    let want = norm(library);
    let Some(folders) = list.get("libraryfolders") else {
        return false;
    };
    folders.entries().iter().any(|(_, lib)| match lib {
        Vdf::Str(path) => norm(Path::new(path)) == want,
        Vdf::Map(_) => {
            lib.get("path").and_then(Vdf::str).is_some_and(|p| norm(Path::new(p)) == want)
                && lib.get("apps").is_none_or(|apps| apps.get(APP_ID).is_some())
        }
    })
}

/// SpaceEngine.exe where Steam's libraries say it is installed, if anywhere.
fn find_on_steam(steam: &[PathBuf]) -> Option<PathBuf> {
    for dir in steam {
        let Some(list) = read_vdf(&dir.join("steamapps").join("libraryfolders.vdf")) else {
            continue;
        };
        for (_, lib) in list.get("libraryfolders").map(Vdf::entries).unwrap_or_default() {
            let Some(path) = lib.str().or_else(|| lib.get("path").and_then(Vdf::str)) else {
                continue;
            };
            let steamapps = Path::new(path).join("steamapps");
            let Some(manifest) = read_vdf(&steamapps.join(format!("appmanifest_{APP_ID}.acf"))) else {
                continue;
            };
            let Some(installdir) = manifest.get("AppState").and_then(|a| a.get("installdir")).and_then(Vdf::str) else {
                continue;
            };
            let exe = steamapps.join("common").join(installdir).join("system").join(EXE);
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Where Steam has another app installed (its `steamapps/common` folder), if anywhere: the sandbox reads Garry's Mod's
/// files from there (`sandbox/assets.rs`). Read only, as everything here.
pub(crate) fn steam_app_dir(app_id: &str) -> Option<PathBuf> {
    for dir in steam_dirs() {
        let Some(list) = read_vdf(&dir.join("steamapps").join("libraryfolders.vdf")) else {
            continue;
        };
        for (_, lib) in list.get("libraryfolders").map(Vdf::entries).unwrap_or_default() {
            let Some(path) = lib.str().or_else(|| lib.get("path").and_then(Vdf::str)) else {
                continue;
            };
            let steamapps = Path::new(path).join("steamapps");
            let Some(manifest) = read_vdf(&steamapps.join(format!("appmanifest_{app_id}.acf"))) else {
                continue;
            };
            let Some(installdir) = manifest.get("AppState").and_then(|a| a.get("installdir")).and_then(Vdf::str) else {
                continue;
            };
            let root = steamapps.join("common").join(installdir);
            if root.is_dir() {
                return Some(root);
            }
        }
    }
    None
}

/// Steam's folders: from the registry, then where Steam installs by default.
fn steam_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    dirs.extend(win::steam_dirs());
    dirs.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(norm(d)));
    dirs
}

/// A path for comparing: absolute where it exists, backslashes, no trailing one, lower case.
fn norm(p: &Path) -> String {
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = p.to_string_lossy().replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.trim_end_matches('\\').to_lowercase()
}

/// An MZ header whose PE header is where it says.
fn windows_program(b: &[u8]) -> bool {
    if b.len() < 0x40 || &b[..2] != b"MZ" {
        return false;
    }
    let at = u32::from_le_bytes([b[0x3c], b[0x3d], b[0x3e], b[0x3f]]) as usize;
    b.get(at..at + 4) == Some(b"PE\0\0".as_slice())
}

/// A UTF-16 string value from an exe's version resource (`StringFileInfo` key `key`).
fn version_string(exe: &[u8], key: &str) -> Option<String> {
    let pat: Vec<u8> = key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let at = exe.windows(pat.len()).rposition(|w| w == pat.as_slice())? + pat.len();
    // The value starts at the next 4-byte boundary.
    let start = at.next_multiple_of(4);
    let words: Vec<u16> = exe.get(start..start + 512)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&w| w != 0).collect();
    Some(String::from_utf16_lossy(&words))
}

/// Valve's KeyValues text (`.vdf`, `.acf`): quoted keys with a string or a `{ }` block each.
#[derive(Debug, Clone, PartialEq)]
enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    /// A key's value (keys are case-insensitive).
    fn get(&self, key: &str) -> Option<&Vdf> {
        self.entries().iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(m) => m,
            Vdf::Str(_) => &[],
        }
    }
}

fn read_vdf(path: &Path) -> Option<Vdf> {
    parse_vdf(&std::fs::read_to_string(path).ok()?)
}

enum Tok {
    S(String),
    Open,
    Close,
}

fn parse_vdf(text: &str) -> Option<Vdf> {
    let mut toks = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            other => s.push(other),
                        },
                        ch => s.push(ch),
                    }
                }
                toks.push(Tok::S(s));
            }
            '{' => toks.push(Tok::Open),
            '}' => toks.push(Tok::Close),
            '/' if chars.peek() == Some(&'/') => {
                while chars.next_if(|&n| n != '\n').is_some() {}
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = String::from(c);
                while let Some(n) = chars.next_if(|&n| !n.is_whitespace() && !matches!(n, '{' | '}' | '"')) {
                    s.push(n);
                }
                toks.push(Tok::S(s));
            }
        }
    }
    let mut i = 0;
    block(&toks, &mut i, false).map(Vdf::Map)
}

fn block(t: &[Tok], i: &mut usize, nested: bool) -> Option<Vec<(String, Vdf)>> {
    let mut m = Vec::new();
    loop {
        match t.get(*i) {
            None => return (!nested).then_some(m),
            Some(Tok::Close) => {
                *i += 1;
                return nested.then_some(m);
            }
            Some(Tok::Open) => return None,
            Some(Tok::S(k)) => {
                let k = k.clone();
                *i += 1;
                match t.get(*i)? {
                    Tok::S(v) => {
                        m.push((k, Vdf::Str(v.clone())));
                        *i += 1;
                    }
                    Tok::Open => {
                        *i += 1;
                        let sub = block(t, i, true)?;
                        m.push((k, Vdf::Map(sub)));
                    }
                    Tok::Close => return None,
                }
            }
        }
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDOK, IDRETRY, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OKCANCEL, MB_RETRYCANCEL, MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE,
        MessageBoxW,
    };

    pub const ERROR: MESSAGEBOX_STYLE = MB_ICONERROR;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// A message box in front of everything; returns its button.
    pub fn message(text: &str, style: MESSAGEBOX_STYLE) -> i32 {
        let (text, title) = (wide(text), wide(super::TITLE));
        // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), style | MB_SETFOREGROUND | MB_TOPMOST) }
    }

    pub fn ok_cancel(text: &str) -> bool {
        message(text, MB_OKCANCEL | MB_ICONINFORMATION) == IDOK
    }

    pub fn retry(text: &str) -> bool {
        message(text, MB_RETRYCANCEL | MB_ICONWARNING) == IDRETRY
    }

    /// Windows' Open dialog for SpaceEngine.exe, starting in `start`.
    pub fn pick_exe(start: Option<&Path>) -> Option<PathBuf> {
        let filter: Vec<u16> = ["SpaceEngine.exe", "SpaceEngine.exe", "Programs (*.exe)", "*.exe"]
            .iter()
            .flat_map(|s| s.encode_utf16().chain([0]))
            .chain([0])
            .collect();
        let title = wide("Select your SpaceEngine.exe (from Steam)");
        let dir = start.map(|p| wide(&p.to_string_lossy()));
        let mut file = vec![0u16; 4096];
        let mut ofn = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: file.as_mut_ptr(),
            nMaxFile: file.len() as u32,
            lpstrInitialDir: dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            lpstrTitle: title.as_ptr(),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT,
            ..Default::default()
        };
        // SAFETY: every pointer in `ofn` points at a NUL-terminated buffer that outlives the call, and
        // `nMaxFile` is the length of the one it writes.
        let ok = unsafe { GetOpenFileNameW(&mut ofn) } != 0;
        let end = file.iter().position(|&c| c == 0).unwrap_or(0);
        (ok && end > 0).then(|| PathBuf::from(OsString::from_wide(&file[..end])))
    }

    fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let (k, v) = (wide(key), wide(value));
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: the key and value names are NUL-terminated; `size` is the buffer's size in bytes.
        let r = unsafe { RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        if r != 0 {
            return None;
        }
        let n = (size as usize / 2).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string())
    }

    /// Where the registry says Steam is.
    pub fn steam_dirs() -> Vec<PathBuf> {
        [
            (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
        ]
        .into_iter()
        .filter_map(|(root, key, value)| reg_string(root, key, value))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake exe: an MZ and PE header and a version resource with these values.
    fn fake_exe(company: &str, product: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x80];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c] = 0x40;
        b[0x40..0x44].copy_from_slice(b"PE\0\0");
        for (k, v) in [("CompanyName", company), ("ProductName", product), ("LegalCopyright", "Copyright (C) 2011-2026 Cosmographic Software"), ("ProductVersion", "0.991.50.2155")] {
            b.extend(k.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
            while b.len() % 4 != 0 {
                b.push(0);
            }
            b.extend(v.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
            b.extend([0u8; 4]);
        }
        b.extend([0u8; 600]);
        b
    }

    struct Fake {
        top: PathBuf,
        exe: PathBuf,
        steam: PathBuf,
    }

    /// A Steam folder and a second library holding SpaceEngine, in a scratch folder.
    fn fake_steam(name: &str) -> Fake {
        let top = std::env::temp_dir().join(format!("se_ownership_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        let (steam, lib) = (top.join("Steam"), top.join("Lib"));
        let root = lib.join("steamapps").join("common").join("SpaceEngine");
        std::fs::create_dir_all(root.join("system")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(steam.join("config")).unwrap();
        let exe = root.join("system").join(EXE);
        std::fs::write(&exe, fake_exe("SpaceEngine", "SpaceEngine")).unwrap();
        let esc = |p: &Path| p.display().to_string().replace('\\', "\\\\");
        std::fs::write(
            lib.join("steamapps").join("appmanifest_314650.acf"),
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"314650\"\n\t\"LauncherPath\"\t\t\"{}\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"SpaceEngine\"\n\t\"LastOwner\"\t\t\"76561190000000001\"\n}}\n", esc(&steam.join("steam.exe"))),
        )
        .unwrap();
        std::fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"228980\"\t\t\"1\"\n\t\t}}\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"314650\"\t\t\"4308777839\"\n\t\t}}\n\t}}\n}}\n",
                esc(&steam),
                esc(&lib)
            ),
        )
        .unwrap();
        std::fs::write(steam.join("config").join("loginusers.vdf"), "\"users\"\n{\n\t\"76561190000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"someone\"\n\t}\n}\n").unwrap();
        Fake { top, exe, steam }
    }

    #[test]
    fn parses_valve_keyvalues() {
        let v = parse_vdf("// note\n\"a\"\n{\n\t\"Path\"\t\t\"D:\\\\Steam Library\"\n\t\"b\" { \"c\" \"1\" }\n}\n").unwrap();
        let a = v.get("A").unwrap();
        assert_eq!(a.get("path").and_then(Vdf::str), Some("D:\\Steam Library"));
        assert_eq!(a.get("b").and_then(|b| b.get("c")).and_then(Vdf::str), Some("1"));
        assert!(parse_vdf("\"a\" { \"b\" \"c\"").is_none());
        assert!(parse_vdf("\"a\" }").is_none());
    }

    #[test]
    fn version_resource_value() {
        let exe = fake_exe("SpaceEngine", "SpaceEngine");
        assert!(windows_program(&exe));
        assert_eq!(version_string(&exe, "ProductVersion").as_deref(), Some("0.991.50.2155"));
        assert_eq!(version_string(&exe, "CompanyName").as_deref(), Some("SpaceEngine"));
        assert!(!windows_program(b"not a program at all, just some text here, long enough for the header"));
    }

    #[test]
    fn a_steam_install_passes() {
        let f = fake_steam("pass");
        let v = verify(&f.exe, &[]).expect("found through the manifest's LauncherPath");
        assert_eq!(v.version, "0.991.50.2155");
        assert_eq!(norm(&v.root), norm(f.exe.parent().unwrap().parent().unwrap()));
        assert_eq!(find_on_steam(std::slice::from_ref(&f.steam)).map(|p| norm(&p)), Some(norm(&f.exe)));
        let _ = std::fs::remove_dir_all(&f.top);
    }

    #[test]
    fn what_fails() {
        let fails = |name: &str, spoil: &dyn Fn(&Fake)| {
            let f = fake_steam(name);
            spoil(&f);
            let r = verify(&f.exe, &[]);
            let _ = std::fs::remove_dir_all(&f.top);
            assert!(r.is_err(), "{name} passed");
        };
        let lib = |f: &Fake| f.top.join("Lib").join("steamapps");
        fails("other_program", &|f| std::fs::write(&f.exe, fake_exe("Someone", "Something")).unwrap());
        fails("no_manifest", &|f| std::fs::remove_file(lib(f).join("appmanifest_314650.acf")).unwrap());
        fails("not_installed", &|f| {
            let p = lib(f).join("appmanifest_314650.acf");
            std::fs::write(&p, std::fs::read_to_string(&p).unwrap().replace("\"StateFlags\"\t\t\"4\"", "\"StateFlags\"\t\t\"2\"")).unwrap();
        });
        fails("library_unlisted", &|f| {
            let p = f.steam.join("steamapps").join("libraryfolders.vdf");
            std::fs::write(&p, std::fs::read_to_string(&p).unwrap().replace("314650", "999")).unwrap();
        });
        fails("owner_never_signed_in", &|f| std::fs::write(f.steam.join("config").join("loginusers.vdf"), "\"users\"\n{\n}\n").unwrap());
        fails("no_data", &|f| std::fs::remove_dir_all(f.exe.parent().unwrap().parent().unwrap().join("data")).unwrap());
        // A copy outside a Steam library.
        let f = fake_steam("moved");
        let elsewhere = f.top.join("Copy").join("system");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::copy(&f.exe, elsewhere.join(EXE)).unwrap();
        let r = verify(&elsewhere.join(EXE), std::slice::from_ref(&f.steam));
        let _ = std::fs::remove_dir_all(&f.top);
        assert!(r.is_err());
    }

    /// The user's own install passes (skipped where it isn't).
    #[test]
    fn the_users_install_passes() {
        let exe = Path::new(r"D:\SteamLibrary\steamapps\common\SpaceEngine\system\SpaceEngine.exe");
        if !exe.is_file() {
            return;
        }
        let v = verify(exe, &steam_dirs()).expect("the user's SpaceEngine");
        assert!(v.version.starts_with("0.99"), "{}", v.version);
        assert!(find_on_steam(&steam_dirs()).is_some());
    }
}
