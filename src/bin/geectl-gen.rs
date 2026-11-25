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
//! bash, zsh, and fish only.
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

use std::fs;
use std::path::PathBuf;

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

fn render_man(cmd: &clap::Command, title: &str, path: &PathBuf) {
    let mut rendered = Vec::new();
    clap_mangen::Man::new(cmd.clone())
        .title(title)
        .section("1")
        .render(&mut rendered)
        .expect("render man page");
    let page = String::from_utf8(rendered).expect("man page is UTF-8");
    fs::write(path, page).expect("write man page");
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let cmd = cli_command();

    let man_dir = root.join("man");
    fs::create_dir_all(&man_dir).expect("create man dir");
    render_man(&cmd, "geectl", &man_dir.join("geectl.1"));
    let leaves = leaf_commands(&cmd);
    for leaf in &leaves {
        // The clone is renamed to its dashed page name, but the synopsis
        // must keep the spaced invocation (`geectl remote cat`): renaming
        // clears the built bin name, so it is set back explicitly. The
        // version is set for the .TH header and VERSION section, but the
        // version flag is disabled: subcommands do not accept --version
        // (the real CLI rejects it), so the page must not advertise it.
        let page = leaf
            .command
            .clone()
            .name(leaf.dashed.clone())
            .bin_name(leaf.spaced.clone())
            .version(env!("CARGO_PKG_VERSION"))
            .disable_version_flag(true);
        render_man(
            &page,
            &leaf.dashed,
            &man_dir.join(format!("{}.1", leaf.dashed)),
        );
    }

    let completions_dir = root.join("completions");
    fs::create_dir_all(&completions_dir).expect("create completions dir");
    let mut comp_cmd = cmd;
    for (shell, ext) in [
        (Shell::Bash, "bash"),
        (Shell::Zsh, "zsh"),
        (Shell::Fish, "fish"),
    ] {
        let path = completions_dir.join(format!("geectl.{ext}"));
        let mut file = fs::File::create(&path).expect("create completion file");
        clap_complete::generate(shell, &mut comp_cmd, "geectl", &mut file);
    }

    println!(
        "wrote {} man pages and 3 completions under {}",
        leaves.len() + 1,
        root.display()
    );
}
