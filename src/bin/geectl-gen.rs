//! Generator for man pages and shell completions.
//!
//! Built only with `--features gen`; it shares the clap `Command` definition
//! with the shipped binary (see [`geectl::cli::cli_command`]) so the two can
//! never drift. Output is written under `{man,completions}` at the repo root
//! and is checked in; CI fails if a clap change leaves the committed assets
//! stale (the assets-drift job).
//!
//! Layout: one man page per leaf command (`geectl-remote-cat.1`, ...), plus
//! the root `geectl.1` page carrying the global flags. Completions cover
//! bash, zsh, and fish only. Regeneration prunes stale pages: a `.1` file
//! under `man/` with no corresponding leaf is removed, so renaming a leaf
//! leaves no orphan page behind.
//!
//! Two rpmcrab patterns are deliberately not mirrored here. There is no
//! hidden-arg filter: geectl has no hidden args (verified by grep over the
//! CLI definition), so there is nothing to strip. There is no appended
//! `EXIT STATUS` section: geectl has no frozen exit-code contract, so there
//! is no frozen table to document.
//!
//! Usage: `cargo run --features gen --bin geectl-gen -- <out-dir>`
//! (defaults to `.` when run from the repo root).

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use clap_complete::Shell;
use geectl::cli::cli_command;

/// One leaf command, with its dashed man-page name and spaced invocation.
struct Leaf {
    dashed: String,
    spaced: String,
    command: clap::Command,
}

/// Every leaf of the command tree, in definition order.
fn leaf_commands(cmd: &clap::Command) -> Vec<Leaf> {
    fn walk(cmd: &clap::Command, path: &[String], out: &mut Vec<Leaf>) {
        let subs: Vec<&clap::Command> = cmd.get_subcommands().collect();
        if subs.is_empty() {
            out.push(Leaf {
                dashed: format!("geectl-{}", path.join("-")),
                spaced: format!("geectl {}", path.join(" ")),
                command: cmd.clone(),
            });
            return;
        }
        for sub in subs {
            let mut next = path.to_vec();
            next.push(sub.get_name().to_string());
            walk(sub, &next, out);
        }
    }
    let mut out = Vec::new();
    walk(cmd, &[], &mut out);
    out
}

/// Render one man page. The file name is derived from the title here, so
/// the call site never re-derives it: the root page is `geectl.1`, a leaf
/// page is `<dashed>.1`.
fn render_man(man_dir: &Path, cmd: clap::Command, title: &str) -> Result<(), Box<dyn Error>> {
    let mut rendered = Vec::new();
    clap_mangen::Man::new(cmd)
        .title(title)
        .section("1")
        .render(&mut rendered)?;
    let page = String::from_utf8(rendered)?;
    fs::write(man_dir.join(format!("{title}.1")), page)?;
    Ok(())
}

/// Remove `.1` files under `man_dir` with no page in `expected`, so a
/// renamed leaf leaves no orphan page behind. Other files are left alone.
fn prune_stale_man_pages(man_dir: &Path, expected: &HashSet<String>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(man_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".1") && !expected.contains(&name) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn run(root: PathBuf) -> Result<(), Box<dyn Error>> {
    let cmd = cli_command();

    let man_dir = root.join("man");
    fs::create_dir_all(&man_dir)?;
    // The root command is cloned once here: the same tree feeds the
    // completions below, so it cannot be moved into the render.
    render_man(&man_dir, cmd.clone(), "geectl")?;
    let leaves = leaf_commands(&cmd);
    let page_count = leaves.len() + 1;
    let mut expected: HashSet<String> = HashSet::with_capacity(page_count);
    expected.insert("geectl.1".to_string());
    for leaf in leaves {
        // The clone is renamed to its dashed page name, but the synopsis
        // must keep the spaced invocation (`geectl remote cat`): renaming
        // clears the built bin name, so it is set back explicitly. The
        // version is set for the .TH header and VERSION section, but the
        // version flag is disabled: subcommands do not accept --version
        // (the real CLI rejects it), so the page must not advertise it.
        let page = leaf
            .command
            .name(leaf.dashed.clone())
            .bin_name(leaf.spaced.clone())
            .version(env!("CARGO_PKG_VERSION"))
            .disable_version_flag(true);
        expected.insert(format!("{}.1", leaf.dashed));
        render_man(&man_dir, page, &leaf.dashed)?;
    }
    prune_stale_man_pages(&man_dir, &expected)?;

    let completions_dir = root.join("completions");
    fs::create_dir_all(&completions_dir)?;
    let mut comp_cmd = cmd;
    for (shell, ext) in [
        (Shell::Bash, "bash"),
        (Shell::Zsh, "zsh"),
        (Shell::Fish, "fish"),
    ] {
        let path = completions_dir.join(format!("geectl.{ext}"));
        let mut file = fs::File::create(&path)?;
        clap_complete::generate(shell, &mut comp_cmd, "geectl", &mut file);
    }

    println!(
        "wrote {page_count} man pages and 3 completions under {}",
        root.display()
    );
    Ok(())
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if let Err(err) = run(root) {
        eprintln!("geectl-gen: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn write_file(dir: &Path, name: &str) {
        fs::write(dir.join(name), "page").unwrap();
    }

    #[test]
    fn prune_removes_stale_man_pages_and_keeps_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let man_dir = tmp.path().join("man");
        fs::create_dir_all(&man_dir).unwrap();
        write_file(&man_dir, "geectl.1");
        write_file(&man_dir, "geectl-remote-cat.1");
        write_file(&man_dir, "geectl-old-leaf.1");
        write_file(&man_dir, "notes.txt");
        let expected: HashSet<String> = ["geectl.1", "geectl-remote-cat.1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        prune_stale_man_pages(&man_dir, &expected).unwrap();
        assert!(!man_dir.join("geectl-old-leaf.1").exists());
        assert!(man_dir.join("geectl.1").exists());
        assert!(man_dir.join("geectl-remote-cat.1").exists());
        assert!(man_dir.join("notes.txt").exists());
    }

    #[test]
    fn render_man_writes_a_page_named_after_the_title() {
        let tmp = tempfile::tempdir().unwrap();
        let man_dir = tmp.path().join("man");
        fs::create_dir_all(&man_dir).unwrap();
        render_man(&man_dir, cli_command(), "geectl").unwrap();
        let page = fs::read_to_string(man_dir.join("geectl.1")).unwrap();
        assert!(page.contains("geectl"));
    }

    #[test]
    fn run_writes_pages_and_prunes_orphans_on_a_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        run(root.clone()).unwrap();
        let expected_pages = leaf_commands(&cli_command()).len() + 1;
        assert_eq!(
            fs::read_dir(root.join("man")).unwrap().count(),
            expected_pages
        );
        write_file(&root.join("man"), "geectl-old-leaf.1");
        run(root.clone()).unwrap();
        assert!(!root.join("man").join("geectl-old-leaf.1").exists());
        assert_eq!(
            fs::read_dir(root.join("man")).unwrap().count(),
            expected_pages
        );
        assert_eq!(fs::read_dir(root.join("completions")).unwrap().count(), 3);
    }
}
