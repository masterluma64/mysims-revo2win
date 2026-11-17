# revo2win

Converts MySims models between the two formats the games shipped them in:

- **RevoModel** (`RMDL`), used by the Wii releases of MySims, MySims Kingdom and MySims Agents;
- **WindowsModel** (`WMDL`), used by the Cozy Bundle releases of MySims and MySims Kingdom on PC.

Both directions work, for static and skinned models, and between games: a Wii model of any of
the three games can be written for either PC release, and a PC model for any of the Wii ones.

RevoModel had no complete public description, so [docs/revomodel.md](docs/revomodel.md)
documents it as far as this tool reads it.

## Building

Needs Rust 1.99 or newer.

```
cargo build --release
```

The program is `target/release/revo2win`.

## Use

```
revo2win to-win  <files, packages or folders>... [-o <output>] [--game mysims|kingdom]
revo2win to-revo <files or folders>... [-o <output>] [--game mysims|kingdom|agents]
revo2win info    <files>...
```

`to-pc` and `to-wii` work as aliases.

Each output goes next to its input unless `-o` says otherwise. With one input file, `-o` can be
the output file; otherwise it is a folder, and folders given as input are mirrored inside it.
Files named by resource key keep their key and get the other format's type id, so
`0x00000000!0x00a94a37dbe5a6db.0xf9e50586` becomes `0x00000000!0x00a94a37dbe5a6db.0xb359c791`.
Other names get `.wmdl` or `.rmdl`. Existing files are only overwritten with `--force`.

The game a file is from is guessed from its contents (Agents has its own type ids; MySims keys
materials by the model's group), and can be set with `--from`. Without `--game`, the output is
for the same game, except that Agents models go to Kingdom, the PC game closest to it.

Examples:

```
revo2win to-win Wii/Kingdom/CatchAll -o PC/Kingdom
revo2win to-revo "0x00000000!0x00c9c8fd11211bae.0xb359c791" --game agents
revo2win info "0x00000000!0x00c9c8fd11211bae.0xb359c791"
```

`to-win` also reads the Wii games' `.package` files directly, whether named or found in a
folder. The models in a package go to a folder named after it, as if it had been extracted in
place: `Characters/CAS-kInitial.package` gives `Characters/CAS-kInitial/` with one file per
model, named by its key (`0x86769daf!0x0000000000000001.0xb359c791`). MySims gives every model
in a package instance 1, so the group is what tells them apart. A model stored more than once
in the same package is written once. Everything else in a package is left alone.

```
revo2win to-win "MySims Agents/DATA/files" -o PC/Agents
revo2win to-win GameData/Characters/CAS-kInitial.package
```

Run on the PAL discs, this converts every model: 5,868 in MySims, 10,448 in Kingdom and 3,127
in Agents.

## What converts, and what does not

Everything a model file holds is carried over: meshes, material references, bounds, positions,
normals, vertex colours, all UV sets, skinning (bones, weights and inverse bind matrices) and the
model's extra parameters. Converting a file to the other format and back gives the same model.

Some things need care:

- **Materials and textures are separate files** and are not converted. The output refers to the
  same material keys, with the type ids of the game it is written for.
- **MySims material names.** The Wii release of MySims names its materials with 64-bit FNV
  hashes and the PC release with 32-bit ones, so a model moved between them points at materials
  that are not there. Given the names, with `--names <file>` (one per line), the references are
  rehashed; those without a name are kept and counted in a note. Kingdom and Agents use 64-bit
  hashes everywhere.
- **Skin weights.** MySims on PC stores three weights per vertex and takes the fourth as what is
  left; Kingdom stores four. The output follows the target game.
- **Missing normals.** Many Agents meshes have none. Smooth normals are computed for them unless
  `--keep-missing-normals` is given.
- **Strips.** Models written for the Wii are drawn with triangle strips built from the triangle
  list, in batches of at most ten bone matrices, as the games' own files are. The strips will not
  match the original exporter's.
- A second vertex colour, and lines or points in a display list, are dropped; neither appears in
  the shipped files.

## Tests

`cargo test` runs the unit tests. The tests in `tests/corpus.rs` run against the games' own
files and are skipped unless pointed at them:

```
REVO2WIN_RMDL_DIR=<extracted Wii models> REVO2WIN_WMDL_DIR=<Cozy Bundle data folders> cargo test --release -- --ignored
```

The Wii folder needs `mysims`, `kingdom` or `agents` in each file's path, so the game is known.
Against the PAL releases of the three Wii games and both Cozy Bundle installs:

- every Kingdom and Agents RevoModel is re-written byte for byte, and every MySims one but one
  apart from end padding (MySims pads its files inconsistently);
- every WindowsModel is re-written byte for byte;
- every model, in both formats, converts to the other format and back without change;
- of the 4,899 Wii models that also exist on PC, 4,466 convert to the same triangles as the PC
  file, and 2,510 of the 2,861 Kingdom ones to exactly the PC file's contents. The rest were
  changed for the PC release.

## Thanks

MorcuTool by Olivercomet, whose RevoModel reader was the starting point for the format notes.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
