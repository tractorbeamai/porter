// See porter-core's lib.rs for the rationale; same restriction-group
// lints are universally exempt from test code here.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic_in_result_fn,
        clippy::str_to_string,
        clippy::missing_panics_doc,
        clippy::missing_errors_doc,
    )
)]

use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use porter_core::{
    Artifact, AttestInput, BuildOpts, Bump, Changeset, ChangesetSet, Config, append_checksum,
    apply_next_version, build_cli_binary, build_matrix, build_provenance, build_statement,
    compute_next_version, current_versions, release_tags, render_for_actions, sha256_hex, slugify,
    validate_changeset_groups, write_changeset,
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
    /// Emit the GitHub Actions job matrix of every group's artifacts.
    Matrix(MatrixArgs),
    /// Build a release artifact (currently `cli-binary` is implemented).
    #[command(subcommand)]
    Build(BuildCmd),
    /// Emit unsigned SLSA provenance for an artifact: a bare predicate
    /// for `cosign attest` to wrap, or a complete in-toto v1 Statement.
    Attest(AttestArgs),
}

#[derive(Args)]
struct AddArgs {
    /// Bump kind. If omitted, prompts.
    #[arg(long, value_enum)]
    bump: Option<BumpArg>,
    /// Group this change bumps; repeatable. Optional when the repo has a
    /// single group; required (or prompted) otherwise.
    #[arg(long = "group", value_name = "GROUP")]
    groups: Vec<String>,
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
struct ReleaseTagArgs;

#[derive(Args)]
struct ReleaseNotesArgs {
    /// Read the named group's changelog instead of the repo-wide default.
    /// Each group's release gets its own notes.
    #[arg(long)]
    group: Option<String>,
}

#[derive(Args)]
struct MatrixArgs {
    /// Filter to a specific artifact kind (e.g. `oci-image`, `cli-binary`).
    #[arg(long)]
    kind: Option<String>,
    /// Filter to a specific group.
    #[arg(long)]
    group: Option<String>,
    /// Print compact JSON instead of pretty-printed.
    #[arg(long)]
    compact: bool,
}

#[derive(Subcommand)]
enum BuildCmd {
    /// Cross-compile a CLI binary, archive it, and write a checksum line.
    CliBinary(BuildCliBinaryArgs),
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum AttestEmit {
    /// A complete in-toto v1 Statement (subject + predicate). Requires
    /// the artifact so its digest can fill the subject.
    Statement,
    /// Just the SLSA provenance predicate, for `cosign attest --type
    /// slsaprovenance1` to wrap and sign (cosign sets the subject).
    Predicate,
}

#[derive(Args)]
struct AttestArgs {
    /// What to emit. `statement` (the default) is a self-contained
    /// in-toto Statement; `predicate` emits just the SLSA provenance for
    /// `cosign attest --type slsaprovenance1` to wrap and sign.
    #[arg(long, value_enum, default_value_t = AttestEmit::Statement)]
    emit: AttestEmit,
    /// Path to the artifact file to attest. Required for `--emit
    /// statement` (used to compute the subject digest); ignored for
    /// `--emit predicate`, where cosign computes the subject itself.
    artifact: Option<PathBuf>,
    /// Override the subject name in the statement (defaults to the file's basename).
    #[arg(long)]
    subject_name: Option<String>,
    /// Source repo, e.g. `tractorbeamai/porter` (defaults to `GITHUB_REPOSITORY`).
    #[arg(long, env = "GITHUB_REPOSITORY")]
    source_repo: String,
    /// Git ref of the source commit (defaults to `GITHUB_REF`).
    #[arg(long, env = "GITHUB_REF")]
    source_ref: String,
    /// Source commit SHA (defaults to `GITHUB_SHA`).
    #[arg(long, env = "GITHUB_SHA")]
    source_sha: String,
    /// CI run id (defaults to `GITHUB_RUN_ID`).
    #[arg(long, env = "GITHUB_RUN_ID")]
    run_id: String,
    /// CI run attempt (defaults to `GITHUB_RUN_ATTEMPT`).
    #[arg(long, env = "GITHUB_RUN_ATTEMPT")]
    run_attempt: Option<String>,
    /// Workflow ref string (defaults to `GITHUB_WORKFLOW_REF`).
    #[arg(long, env = "GITHUB_WORKFLOW_REF")]
    workflow_ref: Option<String>,
    /// ISO-8601 timestamp the run started.
    #[arg(long)]
    started_on: Option<String>,
    /// ISO-8601 timestamp the run finished. Defaults to now.
    #[arg(long)]
    finished_on: Option<String>,
}

#[derive(Args)]
struct BuildCliBinaryArgs {
    /// Component id of the `cli-binary` artifact to build. Defaults to the
    /// only one if there's exactly one across all groups.
    #[arg(long)]
    name: Option<String>,
    /// Override the artifact's cargo `package` value.
    #[arg(long)]
    package: Option<String>,
    /// Override the binary name. Defaults to the component id.
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
            BumpArg::Patch => Self::Patch,
            BumpArg::Minor => Self::Minor,
            BumpArg::Major => Self::Major,
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
    let (_config_path, config, root) = resolve_config(cli.config.as_deref())?;
    match cli.command {
        Command::Add(args) => cmd_add(&root, &config, args),
        Command::Status(args) => cmd_status(&root, &config, &args),
        Command::Version(args) => cmd_version(&root, &config, &args),
        Command::Release(rel) => match rel {
            ReleaseCmd::Tag(_) => cmd_release_tag(&root, &config),
            ReleaseCmd::Notes(args) => cmd_release_notes(&root, &config, args.group.as_deref()),
        },
        Command::Matrix(args) => cmd_matrix(&root, &config, &args),
        Command::Build(b) => match b {
            BuildCmd::CliBinary(args) => cmd_build_cli_binary(&root, &config, args),
        },
        Command::Attest(args) => cmd_attest(args),
    }
}

fn resolve_config(explicit: Option<&Path>) -> Result<(PathBuf, Config, PathBuf)> {
    let path = if let Some(p) = explicit {
        p.to_path_buf()
    } else {
        let cwd = std::env::current_dir().context("getting cwd")?;
        Config::discover(&cwd).context(
            "could not find porter.toml — pass --config or run from inside a porter repo",
        )?
    };
    let config = Config::load(&path)?;
    let root = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok((path, config, root))
}

fn cmd_add(root: &Path, config: &Config, args: AddArgs) -> Result<()> {
    let bump = match args.bump {
        Some(b) => Bump::from(b),
        None => prompt_bump()?,
    };
    let groups = resolve_add_groups(config, args.groups)?;
    let summary = match args.summary {
        Some(s) => s,
        None => prompt_summary()?,
    };
    let summary = summary.trim().to_owned();
    if summary.is_empty() {
        bail!("changeset summary must not be empty");
    }
    let slug = args.slug.unwrap_or_else(|| slugify(&summary));
    let dir = root.join(&config.changesets.directory);
    let path = write_changeset(&dir, &slug, bump, &groups, &summary)?;
    let rel = path.strip_prefix(root).unwrap_or(&path);
    println!("wrote {}", rel.display());
    Ok(())
}

/// Decide which groups a new changeset targets. With one group the selection
/// is implicit (empty); with several, the flags must name existing groups, or
/// we prompt on a tty.
fn resolve_add_groups(config: &Config, requested: Vec<String>) -> Result<Vec<String>> {
    let known: Vec<&str> = config.groups.iter().map(|g| g.name.as_str()).collect();
    if !requested.is_empty() {
        for g in &requested {
            if config.group(g).is_none() {
                bail!("unknown group {g:?} (groups: {})", known.join(", "));
            }
        }
        return Ok(requested);
    }
    if config.groups.len() == 1 {
        // Single group: the changeset belongs to it implicitly.
        return Ok(Vec::new());
    }
    if !io::stdin().is_terminal() {
        bail!(
            "--group is required when the repo has multiple groups (groups: {})",
            known.join(", ")
        );
    }
    eprint!("group(s), comma-separated [{}]: ", known.join(", "));
    io::stderr().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    let chosen: Vec<String> = s
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if chosen.is_empty() {
        bail!("no group selected");
    }
    for g in &chosen {
        if config.group(g).is_none() {
            bail!("unknown group {g:?} (groups: {})", known.join(", "));
        }
    }
    Ok(chosen)
}

/// One group's view in `porter status`: its current/next version, the
/// changesets that bump it, and the tags it would cut.
struct GroupStatus<'a> {
    name: &'a str,
    current: String,
    next: Option<porter_core::NextVersion>,
    set: ChangesetSet,
    tags: Vec<String>,
}

fn cmd_status(root: &Path, config: &Config, args: &StatusArgs) -> Result<()> {
    let dir = root.join(&config.changesets.directory);
    let set = ChangesetSet::load_from_dir(&dir)?;
    validate_changeset_groups(config, &set)?;
    let currents = current_versions(root, config)?;

    // Per-group view: each group's current/next version, the changesets that
    // bump it, and the tags it would cut.
    let mut statuses = Vec::new();
    for group in &config.groups {
        let current = &currents[&group.name];
        let gset = set.for_group(&group.name);
        let next = compute_next_version(current, &gset)?;
        let tags = next.as_ref().map_or_else(Vec::new, |n| {
            group
                .artifact_components()
                .map(|c| c.tag(&n.next))
                .collect()
        });
        statuses.push(GroupStatus {
            name: &group.name,
            current: current.to_string(),
            next,
            set: gset,
            tags,
        });
    }

    // The rolling Version PR title: a single version reads from the lone
    // bumped group; several bumping groups have no single version.
    let bumped: Vec<&GroupStatus<'_>> = statuses.iter().filter(|s| s.next.is_some()).collect();
    let pr_title = match bumped.as_slice() {
        [] => None,
        [only] => only
            .next
            .as_ref()
            .map(|n| config.release.render_pr_title(&n.next.to_string())),
        _ => Some(config.release.render_pr_title_multi()),
    };

    if args.json {
        let groups_json: Vec<_> = statuses
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "current": s.current,
                    "next": s.next.as_ref().map(|n| n.next.to_string()),
                    "bump": s.next.as_ref().map(|n| n.bump.as_str()),
                    "tags": s.tags,
                    "changesets": s.set.changesets.iter().map(format_changeset_json).collect::<Vec<_>>(),
                })
            })
            .collect();
        let payload = serde_json::json!({
            "groups": groups_json,
            // version.yml consumes this so the title/commit subject is
            // configured in porter.toml, not the workflow. Null when nothing
            // is releasable.
            "pr_title": pr_title,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    for s in &statuses {
        match &s.next {
            Some(n) => println!(
                "{}: {} -> {} ({})",
                s.name,
                s.current,
                n.next,
                n.bump.as_str()
            ),
            None => println!("{}: {} (no pending changesets)", s.name, s.current),
        }
        for c in &s.set.changesets {
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
        "groups": c.groups,
    })
}

fn cmd_version(root: &Path, config: &Config, args: &VersionArgs) -> Result<()> {
    let result = apply_next_version(root, config, args.dry_run)?;
    let Some(r) = result else {
        println!("no pending changesets — nothing to do");
        return Ok(());
    };
    let verb = if args.dry_run { "would bump" } else { "bumped" };
    for g in &r.groups {
        println!(
            "{verb} {}: {} -> {} ({})",
            g.group,
            g.next.previous,
            g.next.next,
            g.next.bump.as_str()
        );
        for p in &g.rewritten_files {
            let rel = p.strip_prefix(root).unwrap_or(p);
            println!("  {}", rel.display());
        }
        if !g.tags.is_empty() {
            println!("  tags: {}", g.tags.join(", "));
        }
    }
    let action = if args.dry_run {
        "would consume"
    } else {
        "consumed"
    };
    println!("{action} {} changeset file(s)", r.consumed_changesets.len());

    if !args.dry_run
        && let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY")
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&summary_path)?;
        writeln!(f, "## porter version\n")?;
        for g in &r.groups {
            writeln!(
                f,
                "- **{}: {} → {}** ({}, {} file(s))",
                g.group,
                g.next.previous,
                g.next.next,
                g.next.bump.as_str(),
                g.rewritten_files.len()
            )?;
        }
        writeln!(f, "- {} changesets consumed", r.consumed_changesets.len())?;
    }
    Ok(())
}

fn cmd_release_tag(root: &Path, config: &Config) -> Result<()> {
    // One tag per published component across every group, at each group's
    // current version. The workflow pushes the ones that don't already exist,
    // so unchanged groups are naturally skipped.
    for tag in release_tags(root, config)? {
        println!("{tag}");
    }
    Ok(())
}

fn cmd_matrix(root: &Path, config: &Config, args: &MatrixArgs) -> Result<()> {
    let versions = current_versions(root, config)?;
    let mut rows = build_matrix(config, &versions);
    if let Some(kind) = args.kind.as_deref() {
        rows.retain(|r| r.kind == kind);
    }
    if let Some(group) = args.group.as_deref() {
        rows.retain(|r| r.group == group);
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
    // Find the matching cli-binary component by id across every group. If
    // neither --name nor a single cli-binary can identify it, error out — we
    // don't want to silently build the wrong target.
    let cli_binaries: Vec<(&str, &str)> = config
        .groups
        .iter()
        .flat_map(|g| &g.components)
        .filter_map(|c| match c.artifact() {
            Some(Artifact::CliBinary { package, .. }) => Some((c.id.as_str(), package.as_str())),
            _ => None,
        })
        .collect();

    let (name, package_default) = match args.name {
        Some(n) => {
            let m = cli_binaries
                .iter()
                .find(|(id, _)| *id == n)
                .with_context(|| format!("no cli-binary component with id {n:?}"))?;
            (m.0.to_owned(), m.1.to_owned())
        }
        None => match cli_binaries.as_slice() {
            [] => bail!("porter.toml has no component with a cli-binary artifact"),
            [only] => (only.0.to_owned(), only.1.to_owned()),
            _ => {
                bail!("porter.toml has multiple cli-binary components; pass --name to disambiguate")
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

fn cmd_attest(args: AttestArgs) -> Result<()> {
    // For a full statement we need the artifact's digest to fill the
    // subject; for a bare predicate cosign computes the subject from the
    // artifact it signs, so the file is unnecessary here.
    let (subject_name, subject_sha256) = match args.emit {
        AttestEmit::Statement => {
            let artifact = args.artifact.as_deref().context(
                "--emit statement requires an artifact path to compute the subject digest",
            )?;
            let sha256 = sha256_hex(artifact)?;
            let name = args.subject_name.unwrap_or_else(|| {
                artifact.file_name().map_or_else(
                    || artifact.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            });
            (name, sha256)
        }
        AttestEmit::Predicate => (String::new(), String::new()),
    };

    // The CLI's compile-time version doubles as the builder version
    // recorded in the provenance; consumers can pin policy against this.
    let porter_version = env!("CARGO_PKG_VERSION").to_owned();

    let finished_on = args.finished_on.or_else(|| Some(porter_core::today_utc()));

    let input = AttestInput {
        subject_name,
        subject_sha256,
        source_repo: args.source_repo,
        source_ref: args.source_ref,
        source_sha: args.source_sha,
        run_id: args.run_id,
        run_attempt: args.run_attempt,
        workflow_ref: args.workflow_ref,
        started_on: args.started_on,
        finished_on,
        porter_version,
    };

    let json = match args.emit {
        AttestEmit::Statement => serde_json::to_string_pretty(&build_statement(&input)?)?,
        AttestEmit::Predicate => serde_json::to_string_pretty(&build_provenance(&input)?)?,
    };
    println!("{json}");
    Ok(())
}

fn cmd_release_notes(root: &Path, config: &Config, group: Option<&str>) -> Result<()> {
    // Notes come from the named group's changelog (each group's release gets
    // its own), or the repo-wide default when no group is given.
    let changelog = match group {
        Some(name) => {
            let g = config
                .group(name)
                .with_context(|| format!("unknown group {name:?}"))?;
            g.changelog_path(&config.release).to_path_buf()
        }
        None => config.release.changelog.clone(),
    };
    let cl_path = root.join(&changelog);
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
            if started {
                break;
            }
            started = true;
            continue;
        }
        if started {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !started {
        return None;
    }
    Some(out.trim_end().to_owned())
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
