# 07 Reading SE's files: VFS, catalogs, models, textures, locales

## Paths in scope
- crates/se_core/src/vfs.rs (SE's .pak archives and folders), sc.rs (catalog script parser),
  catalog.rs (1.6k lines), dds.rs, tiff.rs, model_sm.rs, custom_model.rs, locale.rs, places.rs,
  classname.rs, lens_flare.rs, atm.rs, rings.rs
- crates/se_engine/src/data.rs, i18n.rs, config.rs, models.rs, body_models.rs

## What it does
Reads everything from the SpaceEngine install at run time: .pak (zip) archives, catalog .sc files
(stars, planets, galaxies), DDS/TIFF textures, .sm models, locale strings, place names, config files.
User add-ons can also be in the install, so files may be odd or malformed.

## Likely bug classes
- Panics on bad or unusual input: slicing past the end, unwrap on parse, integer overflow on sizes
  from the file (width*height*bpp), huge allocations from a corrupt header, recursion on nested blocks.
- Wrong results: catalog fields parsed in wrong units, a later definition not overriding an earlier
  one (or the reverse), case sensitivity in names/paths, '/' vs '\\' paths, duplicate names,
  parent links that point to the wrong body, DDS mip/size math for block-compressed formats,
  sRGB vs linear.
- VFS: priority order between folders and .pak files (SE's addons override), zip64 / stored vs
  deflate, reading a file that is in two archives.
- Text encoding: UTF-8 vs Windows-1251 locales, BOMs.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. crates/se_core/examples/ (vfs_find, grep_vfs, tile_info, tif_info)
show how to open the install; scratch crates can parse every catalog file to look for panics.
