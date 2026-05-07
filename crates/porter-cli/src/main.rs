use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use porter_core::{
    BuildOpts, Bump, Changeset, ChangesetSet, Config, append_checksum, apply_next_version,
    build_cli_binary, build_matrix, compute_next_version, current_version, render_for_actions,
    slugify, write_changeset,
};

#[derive(Parser)]
#[command(
    name = "porter",
    version,
    about = "Release-cutting tool for polyglot monorepos",
    long_about = "porter cuts a single vX.Y.Z release that bumps every version-bearing\nfile in a monorepo atomically, emits a changelog, and orchestrates\nartifact builds. See `porter <subcommand> --help`."
)]
struct Cli {
    /// Path to porter.toml. Defaults to the nearest one walking up from the
    /// current directory.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Author a new changeset interactively or from flags.
    Add(AddArgs),
    /// Print pending changesets and the next version.
    Status(StatusArgs),
    /// Apply pending changesets: bump every versioned file and prepend the changelog.
    Version(VersionArgs),
    /// Cut a release: tag, build artifacts, push, and create the GitHub Release.
    #[command(subcommand)]
    Release(ReleaseCmd),
    /// Emit the GitHub Actions job matrix derived from `[[artifacts]]`.
    Matrix(MatrixArgs),
    /// Build a release artifact (currently `cli-binary` is implemented).
    #[command(subcommand)]
    Build(BuildCmd),
}

#[derive(Args)]
struct AddArgs {
    /// Bump kind. If omitted, prompts.
    #[arg(long, value_enum)]
    bump: Option<BumpArg>,
    /// One-line summary. If omitted, reads from stdin or prompts.
    #[arg(long)]
    summary: Option<String>,
    /// Filename slug; derived from summary if omitted.
    #[arg(long)]
    slug: Option<String>,
}

#[derive(Args)]
struct StatusArgs {
    /// Emit machine-readable JSON instead of a human summary.
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct VersionArgs {
    /// Print the diff that would be applied without writing files.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum ReleaseCmd {
    /// Print the tag that the next release would carry.
    Tag(ReleaseTagArgs),
    /// Print the changelog body for the most recent release section.
    Notes(ReleaseNotesArgs),
}

#[derive(Args)]
struct ReleaseTagArgs {}

#[derive(Args)]
struct ReleaseNotesArgs {}

#[derive(Args)]
struct MatrixArgs {
    /// Filter to a specific artifact kind (e.g. `oci-image`, `cli-binary`).
    #[arg(long)]
    kind: Option<String>,
    /// Print compact JSON instead of pretty-printed.
    #[arg(long)]
    compact: bool,
}

#[derive(Subcommand)]
enum BuildCmd {
    /// Cross-compile a CLI binary, archive it, and write a checksum line.
    CliBinary(BuildCliBinaryArgs),
}

#[derive(Args)]
struct BuildCliBinaryArgs {
    /// `[[artifacts]]` `name` to look up. Defaults to the only entry if
    /// there's exactly one.
    #[arg(long)]
    name: Option<String>,
    /// Override the `[[artifacts]].package` value.
    #[arg(long)]
    package: Option<String>,
    /// Override the binary name. Defaults to the artifact `name`.
    #[arg(long)]
    binary: Option<String>,
    /// Rust target triple (e.g. `x86_64-unknown-linux-gnu`).
    #[arg(long)]
    target: String,
    /// Output directory for the tarball. Created if missing.
    #[arg(long, default_value = "dist")]
    dist: PathBuf,
    /// Append a checksum line to `<dist>/checksums.txt`.
    #[arg(long, default_value_t = true)]
    checksum: bool,
    /// `cargo` executable to invoke.
    #[arg(long, default_value = "cargo", env = "CARGO")]
    cargo: String,
}

#[derive(Copy, Clone, ValueEnum)]
enum BumpArg {
    Patch,
    Minor,
    Major,
}

impl From<BumpArg> for Bump {
    fn from(b: BumpArg) -> Self {
        match b {
            BumpArg::Patch => Bump::Patch,
            BumpArg::Minor => Bump::Minor,
            BumpArg::Major => Bump::Major,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // GitHub Actions error annotation; the leading `::error::` is
            // a no-op for terminal users.
            eprintln!("::error::{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let (config_path, config, root) = resolve_config(cli.config.as_deref())?;
    let _ = config_path; // currently unused beyond resolution; kept for diagnostics
    match cli.command {
        Command::Add(args) => cmd_add(&root, &config, args),
        Command::Status(args) => cmd_status(&root, &config, args),
        Command::Version(args) => cmd_version(&root, &config, args),
        Command::Release(rel) => match rel {
            ReleaseCmd::Tag(_) => cmd_release_tag(&root, &config),
            ReleaseCmd::Notes(_) => cmd_release_notes(&root, &config),
        },
        Command::Matrix(args) => cmd_matrix(&config, args),
        Command::Build(b) => match b {
            BuildCmd::CliBinary(args) => cmd_build_cli_binary(&root, &config, args),
        },
    }
}

fn resolve_config(explicit: Option<&Path>) -> Result<(PathBuf, Config, PathBuf)> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().context("getting cwd")?;
            Config::discover(&cwd).context(
                "could not find porter.toml — pass --config or run from inside a porter repo",
            )?
        }
    };
    let config = Config::load(&path)?;
    let root = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok((path, config, root))
}

fn cmd_add(root: &Path, config: &Config, args: AddArgs) -> Result<()> {
    let bump = match args.bump {
        Some(b) => Bump::from(b),
        None => prompt_bump()?,
    };
    let summary = match args.summary {
        Some(s) => s,
        None => prompt_summary()?,
    };
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        bail!("changeset summary must not be empty");
    }
    let slug = args.slug.unwrap_or_else(|| slugify(&summary));
    let dir = root.join(&config.changesets.directory);
    let path = write_changeset(&dir, &slug, bump, &summary)?;
    let rel = path.strip_prefix(root).unwrap_or(&path);
    println!("wrote {}", rel.display());
    Ok(())
}

fn cmd_status(root: &Path, config: &Config, args: StatusArgs) -> Result<()> {
    let dir = root.join(&config.changesets.directory);
    let set = ChangesetSet::load_from_dir(&dir)?;
    let current = current_version(root, config)?;
    let next = compute_next_version(&current, &set)?;

    if args.json {
        let payload = serde_json::json!({
            "current": current.to_string(),
            "next": next.as_ref().map(|n| n.next.to_string()),
            "bump": next.as_ref().map(|n| n.bump.as_str()),
            "changesets": set.changesets.iter().map(format_changeset_json).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("current version: {current}");
    if let Some(n) = next {
        println!("next version:    {} ({})", n.next, n.bump.as_str());
    } else {
        println!("next version:    (none — no pending changesets)");
    }
    println!();
    if set.is_empty() {
        println!("no pending changesets");
    } else {
        println!("{} pending changeset(s):", set.len());
        for c in &set.changesets {
            let rel = c.path.strip_prefix(root).unwrap_or(&c.path);
            let first_line = c.summary.lines().next().unwrap_or("");
            println!(
                "  {bump:<5}  {path}  {summary}",
                bump = c.bump.as_str(),
                path = rel.display(),
                summary = first_line
            );
        }
    }
    Ok(())
}

fn format_changeset_json(c: &Changeset) -> serde_json::Value {
    serde_json::json!({
        "path": c.path,
        "bump": c.bump.as_str(),
        "summary": c.summary,
    })
}

fn cmd_version(root: &Path, config: &Config, args: VersionArgs) -> Result<()> {
    let result = apply_next_version(root, config, args.dry_run)?;
    let Some(r) = result else {
        println!("no pending changesets — nothing to do");
        return Ok(());
    };
    if args.dry_run {
        println!(
            "would bump {} -> {} ({})",
            r.next.previous,
            r.next.next,
            r.next.bump.as_str()
        );
        println!("would rewrite:");
        for p in &r.rewritten_files {
            let rel = p.strip_prefix(root).unwrap_or(p);
            println!("  {}", rel.display());
        }
        println!("would prepend section to {}", r.changelog_path.display());
        println!(
            "would consume {} changeset file(s)",
            r.consumed_changesets.len()
        );
    } else {
        println!(
            "bumped {} -> {} ({})",
            r.next.previous,
            r.next.next,
            r.next.bump.as_str()
        );
        println!("rewrote {} file(s):", r.rewritten_files.len());
        for p in &r.rewritten_files {
            let rel = p.strip_prefix(root).unwrap_or(p);
            println!("  {}", rel.display());
        }
        println!(
            "wrote {} and removed {} changeset file(s)",
            r.changelog_path.display(),
            r.consumed_changesets.len()
        );

        if let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY") {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&summary_path)?;
            writeln!(
                f,
                "## porter version\n\n- **{} → {}** ({})\n- {} files rewritten\n- {} changesets consumed",
                r.next.previous,
                r.next.next,
                r.next.bump.as_str(),
                r.rewritten_files.len(),
                r.consumed_changesets.len()
            )?;
        }
    }
    Ok(())
}

fn cmd_release_tag(root: &Path, config: &Config) -> Result<()> {
    let v = current_version(root, config)?;
    println!("{}{}", config.release.tag_prefix, v);
    Ok(())
}

fn cmd_matrix(config: &Config, args: MatrixArgs) -> Result<()> {
    let mut rows = build_matrix(config);
    if let Some(kind) = args.kind.as_deref() {
        rows.retain(|r| r.kind == kind);
    }
    let value = render_for_actions(&rows);
    let body = if args.compact {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    println!("{body}");
    Ok(())
}

fn cmd_build_cli_binary(root: &Path, config: &Config, args: BuildCliBinaryArgs) -> Result<()> {
    use porter_core::ArtifactConfig;
    // Find the matching `[[artifacts]]` block. If neither --name nor a
    // single cli-binary entry can identify it, error out — we don't want
    // to silently build the wrong target.
    let cli_binaries: Vec<_> = config
        .artifacts
        .iter()
        .filter_map(|a| match a {
            ArtifactConfig::CliBinary {
                name,
                package,
                targets,
            } => Some((name.clone(), package.clone(), targets.clone())),
            _ => None,
        })
        .collect();

    let (name, package_default) = match args.name {
        Some(n) => {
            let m = cli_binaries
                .iter()
                .find(|(name, _, _)| name == &n)
                .with_context(|| format!("no [[artifacts]] cli-binary named {n:?}"))?;
            (m.0.clone(), m.1.clone())
        }
        None => match cli_binaries.as_slice() {
            [] => bail!("porter.toml has no [[artifacts]] of kind cli-binary"),
            [only] => (only.0.clone(), only.1.clone()),
            _ => {
                bail!("porter.toml has multiple cli-binary artifacts; pass --name to disambiguate")
            }
        },
    };

    let package = args.package.unwrap_or(package_default);
    let binary = args.binary.unwrap_or_else(|| name.clone());
    let dist = if args.dist.is_absolute() {
        args.dist.clone()
    } else {
        root.join(&args.dist)
    };

    let opts = BuildOpts {
        manifest_dir: root.to_path_buf(),
        package,
        binary,
        target: args.target.clone(),
        dist: dist.clone(),
        cargo: args.cargo,
    };
    let artifact = build_cli_binary(&opts)?;
    println!(
        "built {} (sha256: {})",
        artifact.tarball.display(),
        artifact.sha256
    );
    if args.checksum {
        let p = append_checksum(&dist, &artifact)?;
        println!("appended to {}", p.display());
    }

    if let Ok(out) = std::env::var("GITHUB_OUTPUT") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)?;
        writeln!(f, "tarball={}", artifact.tarball.display())?;
        writeln!(f, "sha256={}", artifact.sha256)?;
    }
    Ok(())
}

fn cmd_release_notes(root: &Path, config: &Config) -> Result<()> {
    let cl_path = root.join(&config.release.changelog);
    let body = std::fs::read_to_string(&cl_path)
        .with_context(|| format!("reading changelog {}", cl_path.display()))?;
    let section = first_section(&body).context("no release section found in changelog")?;
    println!("{section}");
    Ok(())
}

/// Extract the first `## ...` section body (excluding its heading) from a
/// changelog. Stops at the next `## ` or end of file.
fn first_section(body: &str) -> Option<String> {
    let mut lines = body.lines();
    let mut started = false;
    let mut out = String::new();
    for line in lines.by_ref() {
        if line.starts_with("## ") {
            if !started {
                started = true;
                continue;
            } else {
                break;
            }
        }
        if started {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !started {
        return None;
    }
    Some(out.trim_end().to_string())
}

fn prompt_bump() -> Result<Bump> {
    if !io::stdin().is_terminal() {
        bail!("--bump is required when stdin is not a tty");
    }
    eprint!("bump kind [patch/minor/major]: ");
    io::stderr().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(match s.trim().to_ascii_lowercase().as_str() {
        "patch" | "p" | "" => Bump::Patch,
        "minor" | "mi" => Bump::Minor,
        "major" | "ma" | "breaking" => Bump::Major,
        other => bail!("unknown bump kind: {other:?}"),
    })
}

fn prompt_summary() -> Result<String> {
    if io::stdin().is_terminal() {
        eprint!("summary: ");
        io::stderr().flush()?;
        let mut s = String::new();
        io::stdin().read_line(&mut s)?;
        Ok(s)
    } else {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::first_section;
    use indoc::indoc;

    #[test]
    fn first_section_extracts_top_block() {
        let body = indoc! {"
            # Changelog

            ## 0.2.0 — 2026-05-07

            ### Features

            - Foo.

            ## 0.1.0 — 2026-05-01

            - Initial.
        "};
        let s = first_section(body).unwrap();
        assert!(s.contains("### Features"));
        assert!(s.contains("- Foo."));
        assert!(!s.contains("Initial."));
    }
}
