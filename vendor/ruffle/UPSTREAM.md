# Ruffle, vendored (ARCHITECTURE.md A10)

The Ruffle Flash player's crates that run Elite Dangerous's ship-UI movies (`crates/elite-gfx`). Ruffle is
MIT OR Apache-2.0 (`LICENSE.md`, with its third-party notices); the copy keeps its own `[workspace]` and the
project's root `Cargo.toml` excludes this folder (`crates/elite-gfx` uses the crates by path).

- **Upstream:** https://github.com/ruffle-rs/ruffle, commit `4f19836dc657fb1b72a5c72f3433340f75b84050`
  (2026-11-23, wgpu 30.0.2), taken from the UI0 trial's clone of the official repository
  (`re/elite/work/ui/ruffle_spike/ruffle`), with `git archive` (line endings LF, as upstream's
  `.gitattributes` says).
- **Taken:** `core` (with `core/common`, `core/macros`, `core/build_playerglobal`, `core/assets`), `swf`, `wstr`,
  `flv`, `video` (the trait only), `render` (with `render/wgpu`, `render/naga-agal`, `render/naga-pixelbender`,
  `render/pixel_bender`), `tools/asc` (Ruffle's ActionScript compiler, `asc.jar`, used only by
  `remake_playerglobal.sh` and `crates/elite-gfx/as3/build.sh`), the workspace `Cargo.toml` / `Cargo.lock`,
  `LICENSE.md`, `README.md`, `clippy.toml`, `rustfmt.toml`.
- **Left out:** the desktop player, the web player and browser extensions, the exporter, the scanner, the
  frontend utilities, the test suites and their SWF/FLA files (`tests/`, `swf/tests`, `swf/examples`,
  `render/naga-agal/tests` and `agal-helper`, `render/pixel_bender/assembly_tests`), the canvas and WebGL
  renderers, the software and external video decoders, the fuzzer and the stub report tool, `docs/`.
- **Size:** 13 MB, 1,173 files (of which `core/` 8.4 MB, `tools/asc/asc.jar` 1.3 MB, `core/prebuilt/` 0.75 MB).
- **Features used by `elite-gfx`:** `ruffle_core` with no features: no audio (`audio`, `mp3`, `aac`,
  `nellymoser` off), no video decoders, no `jpegxr`, no `default_font` (Elite's fonts are registered as device
  fonts), no `egui`, no `known_stubs`; `ruffle_render_wgpu` with none (no profiling, no WebGL). `swf`'s default
  `lzma` stays on (core asks for `swf`'s defaults; it costs a small crate, nothing at run time).
- **wgpu:** without its default features (patch d): Ruffle asks only for `wgsl` (its renderer's own WGSL
  shaders) and `naga-ir` (`render/wgpu`: its AGAL and Pixel Bender shaders as naga modules); the backends are
  the host's (the engine's Bevy turns on `vulkan`, `dx12`, `metal`). Ruffle adds nothing else to the engine's
  wgpu: `cargo tree --workspace -e features -i wgpu` has, beyond what the engine had before Ruffle came in,
  only `naga-ir`.
- **What else Ruffle adds to the engine's own crates' features** (`cargo metadata` at git HEAD against the
  working tree, 2026-11-23; no crate's version differs): none that changes existing code. Turned off:
  serde_json `preserve_order` (patch e), wgpu's defaults (patch d). Left, each only adding API or a no-op:
  wgpu `naga-ir`; naga `wgsl-out` and `termcolor` (with codespan-reporting `std`, `termcolor`: the WGSL writer
  and coloured stderr helpers); futures-util `channel`, `io`, `sink`, `memchr`, `futures-io` and
  futures-channel `sink` (the `futures` crate's own modules); hashbrown 0.17 `allocator-api2`, `equivalent`
  (trait impls; the hasher and allocator are unchanged); rand and getrandom `sys_rng` (an OS-random generator
  type; no existing generator changes); either, num-bigint, num-complex, num-integer, scopeguard, slotmap,
  url (and through it tinystr, zerovec `alloc`) `std` / `default` (std trait impls; num-traits already had
  `std`, so float maths is unchanged); simdutf8 `public_imp`, `aarch64_neon` (public module, ARM only);
  ttf-parser, wide, aho-corasick, core-foundation-sys `default` (their default features were already on, or
  macOS only).

## Our patches (`patches/`, applied in order to the upstream files; never edit the copy otherwise)

| patch | lines (+ / -) | why |
|---|---|---|
| `a-text-translator.patch` | 68 / 2 | Scaleform's Translator: a text field's text, plain or HTML, passes through `UiBackend::translate_text` when it is set (and its authored text when it is made). A translation holding one of Flash's HTML tags (`a b br font i img li p span textformat u`; Elite's string table uses `font` and `br`) is set as HTML, laid out again only if its text or formats change; any other `<` is text. Elite's movies hold `$KEY;` tokens the game expands. |
| `b-scaleform-extensions.patch` | 524 / 6 | Scaleform GFx 4's AS3 extension classes as built-ins (`scaleform.gfx.Extensions`, `DisplayObjectEx`, `InteractiveObjectEx`, `TextFieldEx`: they shadow the empty stand-ins the movies carry), from Scaleform's documented behaviour: `Extensions.isScaleform` is true; `noInvisibleAdvance` holds invisible clips' timelines (a new clip still enters its first frame); `TextFieldEx` no-translate (patch a's translator skips the field), vertical alignment, text auto-size (shrink / fit) in `display_object/scaleform_text.rs`. Image substitutions, hit-test disabling, edge anti-aliasing, batching and cursors are accepted and do nothing (no inline images in Ruffle's layout, no mouse in this player). |
| `c-prebuilt-builtins.patch` | 141 / 15 | A normal build needs no Java: Ruffle's built-in ActionScript is carried compiled in `core/prebuilt/` and `core/build.rs` copies it to `OUT_DIR`. The stamp of the sources it was made from (`core/playerglobal_stamp`, FNV-1a over the `.as` files under `core/src/avm*/globals`) is in `core/prebuilt/SOURCES`; the build stops if the sources differ. `build_playerglobal` now writes both built-ins and the stamp, and `remake_playerglobal.sh` (added) runs it. |
| `d-workspace-manifest.patch` | 6 / 39 | The workspace lists only the crates taken, drops profile entries for packages no longer in it, takes wgpu without its default features (only `wgsl`; they reached the engine's wgpu), and lowers three version floors to what the engine's lock has, so adding Ruffle changes no engine crate's version: `wgpu` and `naga` 30.0.2 -> 30.0.1, `url` 2.5.9 -> 2.5.8 (same majors; Ruffle builds and runs the movies with them). |
| `e-serde-json-order.patch` | 2 / 1 | `ruffle_core` no longer turns on serde_json's `preserve_order`: cargo unifies features across the workspace, so it switched every `serde_json::Map` in the engine (and in the Skate 3 simulation crates) from sorted to insertion order, which changed the converter's output and could change behaviour at run time. Ruffle uses the ordered map only in ActionScript 3's `JSON.parse` / `JSON.stringify` (`avm2/globals/json.rs`: `insert` and `iter` only, so it builds unchanged); without it a parsed object's keys enumerate, and a stringified object's keys are written, in sorted order instead of Flash's insertion order. Elite's movies never use `JSON` (no decompiled class names it), so nothing they do changes. |

`core/prebuilt/` (the compiled built-ins) is not in a patch: it is made by `remake_playerglobal.sh`.
`Cargo.lock` is cargo's for the trimmed workspace.

## When to run the scripts

- `bash vendor/ruffle/remake_playerglobal.sh` (needs Java 21: `JAVA_HOME`, default
  `C:\Users\liamw\tools\jdk-21`, set for the script only): after changing any `.as` file under
  `core/src/avm1/globals` or `core/src/avm2/globals` (patch b adds some), and after taking a newer Ruffle. The
  build tells you when it is needed. Commit the new `core/prebuilt/` files.
- `bash vendor/ruffle/check_upstream.sh RUFFLE_CLONE`: proves this folder is upstream at the commit plus the
  patches (prints `identical`). Run it after any change here, and remake the patch files when a patch changes.
- A newer Ruffle: take the same paths whole at the new commit, apply the patches (mend them if they no longer
  apply), update the commit above and in `check_upstream.sh`, run `remake_playerglobal.sh`, rebuild
  `crates/elite-gfx/as3` (`build.sh`) and rerun `elite-gfx`'s proofs.
