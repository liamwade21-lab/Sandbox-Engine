# SpaceEngine planet texture paks

Source: `D:/SteamLibrary/steamapps/common/SpaceEngine/data/textures/planets/*.pak` and `Asteroids/*.pak` (79 paks, plus the loose `Asteroids/asteroids.sml`). Every pak is a ZIP. Analysis used Python `zipfile` to list entries and read single small entries only. Nothing was extracted.

## 1. Tile scheme (cube-face quadtree)

- Layout: `<Body>/<Layer>-<Tag>/` holds `base.<ext>` (Earth-Surface-PBC uses `base_a.jpg` and `base_c.jpg`) and six face folders named `neg_x, pos_x, neg_y, pos_y, neg_z, pos_z`.
- Tile file: `<L>_<a>_<b>.<ext>`. `L` is the level. `a` and `b` each run over `0 .. 2^L - 1`.
- Quadtree check: every face holds exactly `4^L` tiles at level `L`. Mars Surface-PBC gives L0=6, L1=24, L2=96, L3=384, L4=1536, L5=6144 (6 faces × 1024 at L5, max a=b=31). The same holds for every tile pak in the table except Moon-Closeups. Earth-Surface has two variant sets (`_a`, `_c`), so its counts are doubled.
- Axis meaning (from edge continuity, Mars Surface-PBC neg_x L5): the right edge of tile `(a,b)` matches the left edge of `(a,b+1)` (mean abs diff 1.6 to 2.1). It does not match `(a+1,b)` (2.9 to 12.7). So `b` runs along image columns and `a` runs along image rows. The bottom-edge test was less decisive.
- Max level per pak is in the table in section 6. Observed max levels are 0 to 5. Small bodies use levels 0 to 2, mid bodies 3 or 4, and Earth, Mars, Moon, Titan and Earth-Clouds/Lights use 5.
- Moon-Closeups-KL.pak is not a full quadtree. Its levels 5, 6 and 7 hold 15, 22 and 50 tiles on `pos_x` only, with irregular grids (L5: a 0..18, b 0..25; L6: a 0..37, b 0..51; L7: a 0..75, b 0..103).

## 2. Tile size and pixel format

- All tiles are 256×256. Nothing uses a border or overlap (no 258 or 514 sizes). Adjacent edges are continuous at the seams.
- Format per layer type (sampled from level 0, level 1 and the max level of each layer):
  - Surface: JPEG RGB for most bodies. JPEG L (grayscale) for Titan-Surface-RV and Earth `_a`. Iapetus-Surface-KX is PNG 8-bit RGB.
  - Bump: PNG 16-bit grayscale (colour type 0) for most bodies. Io, Europa and Ganymede bumps are JPEG L instead.
  - Clouds: JPEG L (Earth, Mars) or JPEG RGB (Venus-Clouds-SF).
  - Lights: PNG 8-bit RGB.
  - Ocean (Titan-Ocean-RV): PNG 8-bit grayscale.
  - Closeups: PNG 16-bit grayscale bump and JPEG RGB surface.

## 3. Base images

- Every base image is 256×256, a 1:1 aspect ratio. It is not a 2:1 equirectangular map. It is probably a low-resolution whole-body preview, but the projection is unverified.
- Earth-Surface-PBC has two bases: `base_a.jpg` (JPEG L) and `base_c.jpg` (JPEG RGB).

## 4. Layer types and naming

- Layer id = `<Layer>-<Tag>`. Layers seen: Surface, Bump, Clouds, Lights, Ocean, Closeups.
- Tags after the dash (PBC, SE, HD, KX, JVV, BJ, SF, FGN, RV, PS, SSS, KL) are not decoded. They are probably source or author codes; this is a guess.
- Earth-Surface-PBC has per-face variants `_a` (JPEG L) and `_c` (JPEG RGB). Their roles are not verified.
- Earth-Clouds-SE and Mars-Clouds-*-HD are JPEG L. Venus-Clouds-SF is JPEG RGB. Mars and Venus cloud layers have only levels 0 and 1.

## 5. Other files

- No `.cfg` or `.sml` files are inside any pak. Entry extensions across all paks: `.jpg` 82367, `.png` 33543, `.dds` 2 (both in Asteroids/67P.pak).
- Loose `Asteroids/asteroids.sml` is in section 8 and references the asteroid pak files by path (`asteroids/<name>`).
- Rings.pak (6 PNG ring textures) and the Asteroids paks are not tile-based. Their files are in section 7.

## 6. Tile-based paks

| Pak | Body | Layer type | Layer id | Ext | Tile px / format | Max level | Max (a,b) at max level | Variants | Base image(s) |
|---|---|---|---|---|---|---|---|---|---|
| Amalthea-Bump-SE.pak | Amalthea | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 0 | 0x0 | 1 | base.png 256x256 PNG 16-bit gray |
| Amalthea-Surface-SE.pak | Amalthea | Surface | Surface-SE | jpg | 256x256 JPEG RGB | 0 | 0x0 | 1 | base.jpg 256x256 JPEG RGB |
| Ariel-Bump-SF.pak | Ariel | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Ariel-Surface-SF.pak | Ariel | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 2 | 3x3 | 1 | base.jpg 256x256 JPEG RGB |
| Callisto-Bump-KX.pak | Callisto | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Callisto-Surface-JVV.pak | Callisto | Surface | Surface-JVV | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Ceres-Bump-HD.pak | Ceres | Bump | Bump-HD | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Ceres-Surface-FGN.pak | Ceres | Surface | Surface-FGN | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Charon-Bump-KX.pak | Charon | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Charon-Surface-KX.pak | Charon | Surface | Surface-KX | jpg | 256x256 JPEG RGB | 2 | 3x3 | 1 | base.jpg 256x256 JPEG RGB |
| Deimos-Bump-SE.pak | Deimos | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 0 | 0x0 | 1 | base.png 256x256 PNG 16-bit gray |
| Deimos-Surface-PBC.pak | Deimos | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Dione-Bump-KX.pak | Dione | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Dione-Surface-PBC.pak | Dione | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Earth-Bump-PBC.pak | Earth | Bump | Bump-PBC | png | 256x256 PNG 16-bit gray | 4 | 15x15 | 1 | base.png 256x256 PNG 8-bit palette |
| Earth-Clouds-SE.pak | Earth | Clouds | Clouds-SE | jpg | 256x256 JPEG L | 5 | 31x31 | 1 | base.jpg 256x256 JPEG L |
| Earth-Lights-HD.pak | Earth | Lights | Lights-HD | png | 256x256 PNG 8-bit RGB | 5 | 31x31 | 1 | base.png 256x256 PNG 8-bit RGB |
| Earth-Surface-PBC.pak | Earth | Surface | Surface-PBC | jpg | 256x256 JPEG L | 5 | 31x31 | 2 | base_a.jpg 256x256 JPEG L; base_c.jpg 256x256 JPEG RGB |
| Enceladus-Bump-KX.pak | Enceladus | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Enceladus-Surface-PBC.pak | Enceladus | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Europa-Bump-BJ-SE.pak | Europa | Bump | Bump-BJ-SE | jpg | 256x256 JPEG L | 3 | 7x7 | 1 | base.jpg 256x256 JPEG L |
| Europa-Surface-BJ.pak | Europa | Surface | Surface-BJ | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Ganymede-Bump-PBC-SE.pak | Ganymede | Bump | Bump-PBC-SE | jpg | 256x256 JPEG L | 3 | 7x7 | 1 | base.jpg 256x256 JPEG L |
| Ganymede-Surface-PBC.pak | Ganymede | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Hyperion-Bump-SE.pak | Hyperion | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 1 | 1x1 | 1 | base.png 256x256 PNG 16-bit gray |
| Hyperion-Surface-JVV.pak | Hyperion | Surface | Surface-JVV | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Iapetus-Bump-KX.pak | Iapetus | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Iapetus-Surface-KX.pak | Iapetus | Surface | Surface-KX | png | 256x256 PNG 8-bit RGB | 3 | 7x7 | 1 | base.png 256x256 PNG 8-bit RGB |
| Io-Bump-SE.pak | Io | Bump | Bump-SE | jpg | 256x256 JPEG L | 3 | 7x7 | 1 | base.jpg 256x256 JPEG L |
| Io-Lights-SE.pak | Io | Lights | Lights-SE | png | 256x256 PNG 8-bit RGB | 3 | 7x7 | 1 | base.png 256x256 PNG 8-bit RGB |
| Io-Surface-PBC.pak | Io | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 3 | 7x7 | 1 | base.jpg 256x256 JPEG RGB |
| Jupiter-Surface-BJ.pak | Jupiter | Surface | Surface-BJ | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Mars-Bump-SE.pak | Mars | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 4 | 15x15 | 1 | base.png 256x256 PNG 16-bit gray |
| Mars-Clouds-dust-HD.pak | Mars | Clouds | Clouds-dust-HD | jpg | 256x256 JPEG L | 1 | 1x1 | 1 | base.jpg 256x256 JPEG L |
| Mars-Clouds-water-HD.pak | Mars | Clouds | Clouds-water-HD | jpg | 256x256 JPEG L | 1 | 1x1 | 1 | base.jpg 256x256 JPEG L |
| Mars-Surface-PBC.pak | Mars | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 5 | 31x31 | 1 | base.jpg 256x256 JPEG RGB |
| Mercury-Bump-HD.pak | Mercury | Bump | Bump-HD | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 8-bit palette |
| Mercury-Surface-HD.pak | Mercury | Surface | Surface-HD | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Mimas-Bump-KX.pak | Mimas | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Mimas-Surface-KX.pak | Mimas | Surface | Surface-KX | jpg | 256x256 JPEG RGB | 3 | 7x7 | 1 | base.jpg 256x256 JPEG RGB |
| Miranda-Bump-SF.pak | Miranda | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Miranda-Surface-SF.pak | Miranda | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Moon-Bump-KL.pak | Moon | Bump | Bump-KL | png | 256x256 PNG 16-bit gray | 4 | 15x15 | 1 | base.png 256x256 PNG 16-bit gray |
| Moon-Surface-KL.pak | Moon | Surface | Surface-KL | jpg | 256x256 JPEG RGB | 5 | 31x31 | 1 | base.jpg 256x256 JPEG RGB |
| Neptune-Surface-SF.pak | Neptune | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Oberon-Bump-SF.pak | Oberon | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Oberon-Surface-SF.pak | Oberon | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 3 | 7x7 | 1 | base.jpg 256x256 JPEG RGB |
| Phobos-Bump-SE.pak | Phobos | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 0 | 0x0 | 1 | base.png 256x256 PNG 16-bit gray |
| Phobos-Surface-PS.pak | Phobos | Surface | Surface-PS | jpg | 256x256 JPEG L | 4 | 15x15 | 1 | base.jpg 256x256 JPEG L |
| Pluto-Bump-SE.pak | Pluto | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Pluto-Surface-FGN.pak | Pluto | Surface | Surface-FGN | jpg | 256x256 JPEG RGB | 3 | 7x7 | 1 | base.jpg 256x256 JPEG RGB |
| Proteus-Bump-SF.pak | Proteus | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 0 | 0x0 | 1 | base.png 256x256 PNG 16-bit gray |
| Proteus-Surface-SF.pak | Proteus | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 0 | 0x0 | 1 | base.jpg 256x256 JPEG RGB |
| Rhea-Bump-KX.pak | Rhea | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Rhea-Surface-PBC.pak | Rhea | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Saturn-Surface-SSS.pak | Saturn | Surface | Surface-SSS | jpg | 256x256 JPEG RGB | 2 | 3x3 | 1 | base.jpg 256x256 JPEG RGB |
| Tethys-Bump-KX.pak | Tethys | Bump | Bump-KX | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Tethys-Surface-PBC.pak | Tethys | Surface | Surface-PBC | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Titan-Bump-RV.pak | Titan | Bump | Bump-RV | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Titan-Ocean-RV.pak | Titan | Ocean | Ocean-RV | png | 256x256 PNG 8-bit gray | 5 | 31x31 | 1 | base.png 256x256 PNG 8-bit gray |
| Titan-Surface-RV.pak | Titan | Surface | Surface-RV | jpg | 256x256 JPEG L | 5 | 31x31 | 1 | base.jpg 256x256 JPEG L |
| Titania-Bump-SF.pak | Titania | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Titania-Surface-SF.pak | Titania | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 2 | 3x3 | 1 | base.jpg 256x256 JPEG RGB |
| Triton-Bump-SE.pak | Triton | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Triton-Surface-SE.pak | Triton | Surface | Surface-SE | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |
| Umbriel-Bump-SF.pak | Umbriel | Bump | Bump-SF | png | 256x256 PNG 16-bit gray | 2 | 3x3 | 1 | base.png 256x256 PNG 16-bit gray |
| Umbriel-Surface-SF.pak | Umbriel | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 2 | 3x3 | 1 | base.jpg 256x256 JPEG RGB |
| Uranus-Surface-SF.pak | Uranus | Surface | Surface-SF | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Venus-Bump-SE.pak | Venus | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Venus-Clouds-SF.pak | Venus | Clouds | Clouds-SF | jpg | 256x256 JPEG RGB | 1 | 1x1 | 1 | base.jpg 256x256 JPEG RGB |
| Venus-Surface-JVV.pak | Venus | Surface | Surface-JVV | jpg | 256x256 JPEG L | 4 | 15x15 | 1 | base.jpg 256x256 JPEG L |
| Vesta-Bump-SE.pak | Vesta | Bump | Bump-SE | png | 256x256 PNG 16-bit gray | 3 | 7x7 | 1 | base.png 256x256 PNG 16-bit gray |
| Vesta-Surface-HD.pak | Vesta | Surface | Surface-HD | jpg | 256x256 JPEG RGB | 4 | 15x15 | 1 | base.jpg 256x256 JPEG RGB |

## 7. Non-tile paks (Rings, Asteroids)

Moon-Closeups-KL.pak (irregular closeup grid, not a full quadtree):
- Moon/Bump-KL L5: 15 tiles, a 0..18, b 0..25, faces: 1 (pos_x only)
- Moon/Bump-KL L6: 22 tiles, a 0..37, b 0..51, faces: 1 (pos_x only)
- Moon/Bump-KL L7: 50 tiles, a 0..75, b 0..103, faces: 1 (pos_x only)
- Moon/Surface-KL L5: 15 tiles, a 0..18, b 0..25, faces: 1 (pos_x only)
- Moon/Surface-KL L6: 22 tiles, a 0..37, b 0..51, faces: 1 (pos_x only)
- Moon/Surface-KL L7: 50 tiles, a 0..75, b 0..103, faces: 1 (pos_x only)


Rings.pak:
| File | Size / format |
|---|---|
| Chariklo-rings.png | 256x2 PNG 8-bit RGBA |
| J1407b-rings.png | 512x2 PNG 8-bit RGBA |
| Jupiter-rings.png | 1024x2 PNG 8-bit RGBA |
| Neptune-rings.png | 256x2 PNG 8-bit RGBA |
| Saturn-rings.png | 16384x2 PNG 8-bit RGBA |
| Uranus-rings.png | 8868x2 PNG 8-bit RGBA |

Asteroids (not tile-based):
| Pak | File | Size | Format |
|---|---|---|---|
| Asteroids/2014MU69.pak | 2014MU69_albedo.jpg | 1024x1024 | JPEG RGB |
| Asteroids/2014MU69.pak | 2014MU69_bump.png | 1024x1024 | PNG 16-bit gray |
| Asteroids/2014MU69.pak | DetailAsteroid_COL.jpg | 256x256 | JPEG RGB |
| Asteroids/2014MU69.pak | DetailAsteroid_DISP.png | 256x256 | PNG 16-bit gray |
| Asteroids/67P.pak | 67P_albedo.dds | ?x? | DDS (not parsed) |
| Asteroids/67P.pak | 67P_normal.dds | ?x? | DDS (not parsed) |
| Asteroids/67P.pak | LavaFlow002_COL.png | 512x512 | PNG 8-bit palette |
| Asteroids/67P.pak | LavaFlow002_DISP.png | 512x512 | PNG 8-bit gray |
| Asteroids/Eros.pak | Eros_albedo.png | 1024x1024 | PNG 16-bit RGBA |
| Asteroids/Eros.pak | Eros_normal.png | 1024x1024 | PNG 16-bit RGBA |
| Asteroids/Itokawa.pak | GroundKX3_COL.png | 256x256 | PNG 16-bit RGB |
| Asteroids/Itokawa.pak | GroundKX3_DISP.png | 256x256 | PNG 16-bit gray |
| Asteroids/Itokawa.pak | Itokawa_albedo.png | 1024x1024 | PNG 16-bit RGBA |
| Asteroids/Itokawa.pak | Itokawa_normal.png | 1024x1024 | PNG 16-bit RGBA |

## 8. Asteroids/asteroids.sml (verbatim, 92 lines)

```
Material	"Asteroid_default"
{
	DiffColor       (0.14 0.14 0.14)
	SpecColor       (0.05 0.05 0.05)
	SpecPower       60.0
}

Material	"Comet_default"
{
	DiffColor       (0.14 0.14 0.14)
	SpecColor       (0.25 0.25 0.25)
	SpecPower       120.0
}

Material	"Asteroid_1I"
{
    DiffColor       (0.580 0.435 0.333)
    SpecColor       (0.05 0.05 0.05)
    SpecPower       60.0
    DetDiffMap      "asteroids/GroundKX3_COL.png"
    DetBumpMap      "asteroids/GroundKX3_DISP.png"
    DetDiffScale    16
    DetBumpScale    16
    DetDiffMag      0.75
    DetBumpMag      0.5
    DetBumpHeight   1.0
}

Material	"Comet_67P"
{
	DiffColorScale  (0.10 0.10 0.10)
	SpecColor       (0.25 0.25 0.25)
	SpecPower       120.0
	DiffMap         "asteroids/67P_albedo.dds"
	NormMap         "asteroids/67P_normal.dds"
	DetDiffMap      "asteroids/LavaFlow002_COL.png"
	DetBumpMap      "asteroids/LavaFlow002_DISP.png"
	DetDiffScale    512
	DetBumpScale    512
	DetDiffMag      0.75
	DetBumpMag      0.5
	DetBumpHeight   8.0
}

Material	"Asteroid_Eros"
{
    DiffColor       (0.4 0.4 0.4)
    SpecColor       (0.0 0.0 0.0)
    SpecPower       60.0
    DiffMap         "asteroids/Eros_albedo.png"
    NormMap         "asteroids/Eros_normal.png"
    DetDiffMap      "asteroids/GroundKX3_COL.png"
    DetBumpMap      "asteroids/GroundKX3_DISP.png"
    DetDiffScale    30
    DetBumpScale    30
    DetDiffMag      0.5
    DetBumpMag      0.5
    DetBumpHeight   4.0
}

Material	"Asteroid_Itokawa"
{
	DiffColor       (0.6 0.6 0.6)
	SpecColor       (0.0 0.0 0.0)
	SpecPower       60.0
	DiffMap         "asteroids/Itokawa_albedo.png"
	NormMap         "asteroids/Itokawa_normal.png"
	DetDiffMap      "asteroids/GroundKX3_COL.png"
	DetBumpMap      "asteroids/GroundKX3_DISP.png"
	DetDiffScale    4
	DetBumpScale    4
	DetDiffMag      0.75
	DetBumpMag      0.5
	DetBumpHeight   8.0
}

Material	"Asteroid_2014MU69"
{
	DiffColor       (0.1 0.1 0.1)
	SpecColor       (0.0 0.0 0.0)
	SpecPower       60.0
	DiffMap         "asteroids/2014MU69_albedo.jpg"
	BumpMap         "asteroids/2014MU69_bump.png"
	BumpHeight      8.0
	DetDiffMap      "asteroids/DetailAsteroid_COL.jpg"
	DetBumpMap      "asteroids/DetailAsteroid_DISP.png"
	DetDiffScale    128
	DetBumpScale    128
	DetDiffMag      0.5
	DetBumpMag      0.5
	DetBumpHeight   8.0
}
```

## 9. Not done

- Cube-face orientation convention versus the base image was not checked.
- Layer tags and the `_a`/`_c` variant roles were not decoded.
- Bump and Clouds header checks were sampled per layer, not for every tile.
