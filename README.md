# revo2win

Converts MySims models between the Wii and PC versions of the games.

- **RevoModel** is the model format of the Wii releases of MySims, MySims Kingdom and MySims
  Agents.
- **WindowsModel** is the model format of the Cozy Bundle releases of MySims and MySims Kingdom
  on PC.

It works in both directions, for regular and animated (skinned) models, and between games: a Wii
model from any of the three games can be converted for either PC game, and a PC model for any of
the Wii games.

## Download

Get `revo2win-<version>-windows-x64.zip` from the
[releases page](https://github.com/masterluma64/mysims-revo2win/releases), unzip it, and run
`revo2win.exe` from a command prompt or PowerShell. Nothing needs installing.

## Use

```
revo2win to-win  <files, packages or folders>... [-o <output>] [--game mysims|kingdom]
revo2win to-revo <files or folders>... [-o <output>] [--game mysims|kingdom|agents]
revo2win info    <files>...
```

- `to-win` converts Wii models to PC (alias `to-pc`).
- `to-revo` converts PC models to Wii (alias `to-wii`).
- `info` shows what a model file contains.

`to-win` reads the Wii games' `.package` files directly, so you can point it at a whole disc's
`files` folder. Each package's models go into a folder named after the package.

Examples:

```
revo2win to-win "MySims Agents/DATA/files" -o PC/Agents
revo2win to-win GameData/Characters/CAS-kInitial.package
revo2win to-revo "0x00000000!0x00c9c8fd11211bae.0xb359c791" --game agents
```

### Options

| Option | What it does |
|---|---|
| `-o <output>` | Where to write. A file name when converting one file, otherwise a folder. Without it, each output goes next to its input. |
| `--game <game>` | The game to convert for. Without it, the model stays in its own game; Agents models go to Kingdom, which has no Agents PC release to go to. |
| `--from <game>` | The game the input is from, if the guess is wrong. |
| `--names <file>` | Material names, one per line, for MySims (see below). |
| `--keep-missing-normals` | Don't add normals to meshes that have none. |
| `-f`, `--force` | Overwrite files that already exist. |

Files named like the games name them keep their name with the other format's type, so
`0x00000000!0x00a94a37dbe5a6db.0xf9e50586` becomes `0x00000000!0x00a94a37dbe5a6db.0xb359c791`.
Other files get `.wmdl` or `.rmdl`.

## Good to know

- **Only models are converted.** Materials and textures are separate files. A converted model
  uses the same material names, so they need to exist in the game you put it in.
- **MySims material names differ between Wii and PC.** If you have a list of the material
  names, pass it with `--names` so the converted model points at the right materials. Kingdom
  and Agents don't need this.
- **Normals are added where missing.** Many Agents meshes have none; they get smooth normals
  unless you pass `--keep-missing-normals`.

## Building from source

Needs [Rust](https://rustup.rs) 1.99 or newer.

```
cargo build --release
```

The program ends up in `target/release/`.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
