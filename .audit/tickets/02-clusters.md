# Ticket 02: Star clusters

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `02`; write `.audit/gaps/02.txt`; scratch files go in `.audit/work/02/`.

## Scope
Catalog clusters (`Catalogs.pak` cluster files and every key SE reads for them) and procedural `RC`
clusters (main-def.cfg `ClusterGenerator` 978-1015): how a cluster is drawn far away (sprite, point cloud),
how it turns into individual stars as you approach (the member stars become real, selectable stars with
their own names and planetary systems), open vs globular vs embedded/nucleus, member-star generation
(count, mass function, radial profile, King/Plummer model, colours, ages), cluster brightness and its
fade/LOD thresholds, selection and info of clusters and their members, clusters in other galaxies.

## Notes and boundaries
The main session is root-causing one known bug itself: "star cluster sprites (near Pozzo 1) fade out as
you fly up to them, instead of turning into real stars as in SE". Do not root-cause that one, but DO find
and document SE's mechanism for the far-sprite -> member-stars transition (functions, thresholds, how member
stars are generated and named) and list each part of it the port lacks: that evidence is valuable. Also
write what you learn about that mechanism (function addresses, thresholds, member-star generation) to
`.audit/work/02/cluster_mechanism.md`.
Out of this ticket: field stars and star rendering in general (01), nebulae (03).

## Port starting points
`crates/se_engine/src/clusters.rs`, `crates/se_engine/src/procstars.rs`, `crates/se_core/src/{proc_galsys,catalog,star_octree}.rs`, docs `formats/procedural_deepsky.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
