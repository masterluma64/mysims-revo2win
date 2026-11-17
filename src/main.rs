use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};

use revo2win::convert::{Format, Model, Options, Report, material_name_map};
use revo2win::game::{Game, types};
use revo2win::package::{self, Package};
use revo2win::{RevoModel, WindowsModel, detect};

/// Converts MySims models between the Wii's RevoModel and the PC's WindowsModel.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// RevoModel (Wii) to WindowsModel (PC).
    #[command(alias = "to-pc")]
    ToWin(Convert),
    /// WindowsModel (PC) to RevoModel (Wii).
    #[command(alias = "to-wii")]
    ToRevo(Convert),
    /// Describes model files.
    Info {
        /// Model files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Args)]
struct Convert {
    /// Model files, Wii .package files, or folders to search for either.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// Where to write: a file when converting one file, otherwise a folder
    /// (the folders searched are mirrored inside it). Default: next to each
    /// input.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// The game to write for: mysims, kingdom or agents. Default: the game
    /// the input is from (Kingdom for Agents models going to PC).
    #[arg(short, long)]
    game: Option<Game>,

    /// The game the inputs are from, when guessing it from the file is wrong.
    #[arg(long)]
    from: Option<Game>,

    /// A text file of material names, one per line. MySims names materials
    /// with FNV64 on the Wii and FNV32 on PC; with the names, material
    /// references are rehashed.
    #[arg(long)]
    names: Option<PathBuf>,

    /// Leave meshes without normals as they are instead of computing them.
    #[arg(long)]
    keep_missing_normals: bool,

    /// Overwrite output files that already exist.
    #[arg(short, long)]
    force: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::ToWin(args) => convert(&args, Format::Windows),
        Command::ToRevo(args) => convert(&args, Format::Revo),
        Command::Info { files } => info(&files),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// A file to convert, and the folder it was found under (for mirroring).
struct Job {
    path: PathBuf,
    root: Option<PathBuf>,
}

fn collect(inputs: &[PathBuf]) -> anyhow::Result<Vec<Job>> {
    let mut jobs = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let mut stack = vec![input.clone()];
            let mut found = Vec::new();
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
                    let path = entry?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        found.push(path);
                    }
                }
            }
            found.sort();
            jobs.extend(found.into_iter().map(|path| Job { path, root: Some(input.clone()) }));
        } else if input.is_file() {
            jobs.push(Job { path: input.clone(), root: None });
        } else {
            bail!("{} does not exist", input.display());
        }
    }
    Ok(jobs)
}

fn read_names(path: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

/// The output name: the input's, with the extension for the other format.
fn output_name(input: &Path, to: Format, game: Game) -> PathBuf {
    let name = input.file_name().unwrap_or_default().to_string_lossy();
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
    let typed = detect::type_from_file_name(&name).is_some();
    let ext = ext.to_ascii_lowercase();
    let new = match to {
        Format::Windows if typed => format!("{stem}.0x{:08x}", types::WINDOWS_MODEL),
        Format::Windows if ext == "rmdl" => format!("{stem}.wmdl"),
        Format::Windows => format!("{name}.wmdl"),
        Format::Revo if typed => format!("{stem}.0x{:08x}", game.revo_model_type()),
        Format::Revo if ext == "wmdl" => format!("{stem}.rmdl"),
        Format::Revo => format!("{name}.rmdl"),
    };
    input.with_file_name(new)
}

fn convert(args: &Convert, to: Format) -> anyhow::Result<bool> {
    let names = args.names.as_deref().map(read_names).transpose()?;
    if let Some(game) = args.game
        && to == Format::Windows
        && !game.has_windows_release()
    {
        bail!("{game} has no PC release; write for mysims or kingdom");
    }
    let jobs = collect(&args.inputs)?;
    let single = jobs.len() == 1 && jobs[0].root.is_none();
    let mut tally = Tally::default();
    let from = match to {
        Format::Windows => Format::Revo,
        Format::Revo => Format::Windows,
    };

    for job in &jobs {
        let data = std::fs::read(&job.path).with_context(|| format!("reading {}", job.path.display()))?;
        if to == Format::Windows && package::is_package(&data) {
            convert_package(&data, job, args, names.as_deref(), &mut tally)?;
            continue;
        }
        let is_input = match from {
            Format::Revo => RevoModel::is_revo_model(&data),
            Format::Windows => WindowsModel::is_windows_model(&data),
        };
        if !is_input {
            if job.root.is_none() {
                eprintln!("{}: not a {}", job.path.display(), format_name(from));
                tally.failed += 1;
            } else {
                tally.skipped += 1;
            }
            continue;
        }
        convert_file(&data, job, single, to, args, names.as_deref(), &mut tally)?;
    }
    if jobs.len() > 1 || tally.packages > 0 {
        println!("{} converted, {} failed, {} other files skipped", tally.done, tally.failed, tally.skipped);
    }
    Ok(tally.failed == 0)
}

#[derive(Default)]
struct Tally {
    done: usize,
    failed: usize,
    skipped: usize,
    packages: usize,
}

/// Converts every RevoModel in a package. Each goes to a folder named after
/// the package, as if the package had been extracted next to itself.
fn convert_package(
    data: &[u8],
    job: &Job,
    args: &Convert,
    names: Option<&[String]>,
    tally: &mut Tally,
) -> anyhow::Result<()> {
    tally.packages += 1;
    let package = match Package::read(data) {
        Ok(p) => p,
        Err(e) => {
            if job.root.is_some() {
                eprintln!("{}: skipped, not a readable package ({e})", job.path.display());
                tally.skipped += 1;
            } else {
                eprintln!("{}: {e}", job.path.display());
                tally.failed += 1;
            }
            return Ok(());
        }
    };
    let stem = job.path.file_stem().unwrap_or_default();
    let folder = job.path.with_file_name(stem);
    // A package given by itself is mirrored as its own folder under -o.
    let root = job.root.clone().or_else(|| job.path.parent().map(Path::to_path_buf));
    let mut models = package
        .entries
        .iter()
        .filter(|e| matches!(e.key.type_id, types::REVO_MODEL | types::REVO_MODEL_AGENTS))
        .peekable();
    if models.peek().is_none() {
        if job.root.is_none() {
            eprintln!("{}: no models in this package", job.path.display());
        }
        tally.skipped += 1;
        return Ok(());
    }
    // MySims's CAS packages hold some models many times over, byte for byte.
    let mut seen = std::collections::HashMap::new();
    for entry in models {
        let path = folder.join(entry.key.to_string());
        let model = Job { path, root: root.clone() };
        let extracted = package.extract(entry);
        if let Ok(bytes) = &extracted {
            match seen.get(&entry.key) {
                Some(first) if first == bytes => continue,
                Some(_) => {
                    eprintln!("{}: in the package twice, with different contents", model.path.display());
                    tally.failed += 1;
                    continue;
                }
                None => {
                    seen.insert(entry.key, bytes.clone());
                }
            }
        }
        match extracted {
            Ok(bytes) if RevoModel::is_revo_model(&bytes) => {
                convert_file(&bytes, &model, false, Format::Windows, args, names, tally)?;
            }
            Ok(_) => {
                eprintln!("{}: not a RevoModel", model.path.display());
                tally.failed += 1;
            }
            Err(e) => {
                eprintln!("{}: {e}", model.path.display());
                tally.failed += 1;
            }
        }
    }
    Ok(())
}

fn convert_file(
    data: &[u8],
    job: &Job,
    single: bool,
    to: Format,
    args: &Convert,
    names: Option<&[String]>,
    tally: &mut Tally,
) -> anyhow::Result<()> {
    match convert_one(data, &job.path, to, args, names) {
        Ok((bytes, source, target, report)) => {
            let out_path = destination(job, args.output.as_deref(), single, to, target);
            if out_path == job.path {
                eprintln!("{}: the output would replace the input; skipped", job.path.display());
                tally.failed += 1;
                return Ok(());
            }
            if out_path.exists() && !args.force {
                eprintln!("{}: exists (use --force to overwrite)", out_path.display());
                tally.failed += 1;
                return Ok(());
            }
            if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&out_path, bytes).with_context(|| format!("writing {}", out_path.display()))?;
            println!("{} ({source}) -> {} ({target})", job.path.display(), out_path.display());
            for note in report.notes {
                println!("  note: {note}");
            }
            tally.done += 1;
        }
        Err(e) => {
            eprintln!("{}: {e:#}", job.path.display());
            tally.failed += 1;
        }
    }
    Ok(())
}

fn destination(job: &Job, output: Option<&Path>, single: bool, to: Format, target: Game) -> PathBuf {
    let name = output_name(&job.path, to, target);
    match output {
        Some(o) if single && !o.is_dir() => o.to_path_buf(),
        Some(o) => {
            let relative = match &job.root {
                Some(root) => {
                    job.path.parent().and_then(|p| p.strip_prefix(root).ok()).unwrap_or(Path::new(""))
                }
                None => Path::new(""),
            };
            o.join(relative).join(name.file_name().unwrap_or_default())
        }
        None => name,
    }
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Revo => "RevoModel",
        Format::Windows => "WindowsModel",
    }
}

fn convert_one(
    data: &[u8],
    path: &Path,
    to: Format,
    args: &Convert,
    names: Option<&[String]>,
) -> anyhow::Result<(Vec<u8>, Game, Game, Report)> {
    let mut report = Report::default();
    let type_id = path.file_name().and_then(|n| detect::type_from_file_name(&n.to_string_lossy()));
    let (model, source) = match to {
        Format::Windows => {
            let revo = RevoModel::read(data)?;
            let source = args.from.unwrap_or_else(|| detect::revo_game(&revo, type_id));
            (Model::from_revo(&revo, &mut report)?, source)
        }
        Format::Revo => {
            let win = WindowsModel::read(data)?;
            let source = args.from.unwrap_or_else(|| detect::windows_game(&win));
            (Model::from_windows(&win, &mut report)?, source)
        }
    };
    let target = args.game.unwrap_or(match (to, source) {
        (Format::Windows, Game::Agents) => Game::Kingdom,
        _ => source,
    });
    let from = match to {
        Format::Windows => Format::Revo,
        Format::Revo => Format::Windows,
    };
    let options = Options {
        material_instances: names
            .map(|n| material_name_map(n.iter().map(String::as_str), (source, from), (target, to)))
            .unwrap_or_default(),
        keep_missing_normals: args.keep_missing_normals,
    };
    if source == Game::MySims
        && target == Game::MySims
        && options.material_instances.is_empty()
        && names.is_none()
    {
        report.notes.push(
            "MySims material references are FNV64 on the Wii and FNV32 on PC; pass --names to rehash them"
                .into(),
        );
    }
    let bytes = match to {
        Format::Windows => model.to_windows(target, &options, &mut report)?.write(),
        Format::Revo => model.to_revo(target, &options, &mut report)?.write(target),
    };
    Ok((bytes, source, target, report))
}

fn info(files: &[PathBuf]) -> anyhow::Result<bool> {
    let mut ok = true;
    for path in files {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        println!("{}", path.display());
        let type_id = path.file_name().and_then(|n| detect::type_from_file_name(&n.to_string_lossy()));
        let mut report = Report::default();
        let described = if RevoModel::is_revo_model(&data) {
            RevoModel::read(&data).map_err(anyhow::Error::from).and_then(|m| {
                println!("  RevoModel, {} (guessed)", detect::revo_game(&m, type_id));
                describe_revo(&m);
                Ok(Model::from_revo(&m, &mut report)?)
            })
        } else if WindowsModel::is_windows_model(&data) {
            WindowsModel::read(&data).map_err(anyhow::Error::from).and_then(|m| {
                println!(
                    "  WindowsModel {}.{}, {} (guessed)",
                    m.major_version,
                    m.minor_version,
                    detect::windows_game(&m)
                );
                Ok(Model::from_windows(&m, &mut report)?)
            })
        } else {
            Err(anyhow::anyhow!("not a RevoModel or WindowsModel"))
        };
        match described {
            Ok(model) => describe(&model),
            Err(e) => {
                println!("  error: {e:#}");
                ok = false;
            }
        }
    }
    Ok(ok)
}

fn describe_revo(m: &RevoModel) {
    for (i, mesh) in m.meshes.iter().enumerate() {
        let arrays: Vec<String> =
            mesh.arrays.iter().map(|a| format!("{}×{}", attribute_name(a.attribute), a.len())).collect();
        let draws =
            mesh.display_list.iter().filter(|c| matches!(c, revo2win::rmdl::Command::Draw { .. })).count();
        println!("  mesh {i} arrays: {}; {draws} primitives", arrays.join(", "));
        if let Some(skin) = &mesh.skin {
            println!(
                "    skin: {} palette entries over {} bones, rig {}",
                skin.palette.len(),
                skin.bones.len(),
                skin.rig
            );
        }
    }
}

fn attribute_name(a: u8) -> String {
    use revo2win::rmdl::attr;
    match a {
        attr::POSITION => "position".into(),
        attr::NORMAL => "normal".into(),
        attr::COLOR0 => "colour0".into(),
        attr::COLOR1 => "colour1".into(),
        a => match attr::uv_set(a) {
            Some(s) => format!("uv{s}"),
            None => format!("attribute {a}"),
        },
    }
}

fn describe(m: &Model) {
    for e in &m.extras {
        println!("  extra 0x{:08x} = {:?}", e.key, String::from_utf8_lossy(&e.value));
    }
    for (i, r) in m.rigs.iter().enumerate() {
        println!("  rig {i}: {} bones", r.bone_hashes.len());
    }
    for (i, mesh) in m.meshes.iter().enumerate() {
        let v = &mesh.vertices;
        let mut parts = vec![format!("{} vertices", v.len()), format!("{} triangles", mesh.triangles.len())];
        if v.normals.is_some() {
            parts.push("normals".into());
        }
        if v.colors.is_some() {
            parts.push("colours".into());
        }
        for (set, _) in &v.uvs {
            parts.push(format!("uv{set}"));
        }
        if let Some(r) = mesh.rig {
            parts.push(format!("skinned to rig {r}"));
        }
        println!("  mesh {i} material {}: {}", mesh.material, parts.join(", "));
    }
}
