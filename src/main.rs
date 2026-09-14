//! jump — a visual, keyboard-driven way to walk the filesystem that leaves your
//! shell in the directory you picked.

#[cfg(not(windows))]
compile_error!(
    "jump currently only supports Windows; the Unix console backend (libc) is not written yet"
);

mod app;
mod entry;
mod shell;
mod text;
mod win;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use app::{App, Outcome};
use win::Console;

const HELP: &str = "\
jump — walk the filesystem with the arrow keys

USAGE
    jump [OPTIONS] [PATH]

KEYS
    ↑ / ↓              move the selection
    PgUp / PgDn        move by ten
    Home / End         first / last entry
    → or Enter         open the highlighted folder
    ← or Backspace     go up a folder (Backspace edits an active search first)
    type a name        select it, the way Explorer does
    Tab                next match for what you typed
    Esc                quit, taking the shell to the folder you are in
    Ctrl+C             quit without changing directory
    Ctrl+H             show or hide hidden files
    F5 / Ctrl+R        re-read the folder

OPTIONS
    -a, --hidden       start with hidden files shown
        --cd-file <F>  write the chosen directory to <F> (used by the shell wrapper)
        --init <SHELL> print the shell wrapper for powershell, cmd or bash
        --install      add the wrapper to your PowerShell profile
        --uninstall    remove it again
        --selftest     render one frame to stdout and exit (no TUI, for diagnostics)
    -h, --help         show this help
    -V, --version      show the version

Without the shell wrapper, jump prints the chosen directory to stdout on exit.\
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("jump: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> io::Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut start: Option<String> = None;
    let mut cd_file: Option<PathBuf> = None;
    let mut show_hidden = false;
    let mut selftest = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(ExitCode::SUCCESS);
            }
            "-V" | "--version" => {
                println!("jump {}", env!("CARGO_PKG_VERSION"));
                return Ok(ExitCode::SUCCESS);
            }
            "--init" => {
                let shell = args.get(i + 1).map(String::as_str).unwrap_or("powershell");
                shell::print_init(shell)?;
                return Ok(ExitCode::SUCCESS);
            }
            "--install" => {
                shell::install()?;
                return Ok(ExitCode::SUCCESS);
            }
            "--uninstall" => {
                shell::uninstall()?;
                return Ok(ExitCode::SUCCESS);
            }
            "--cd-file" => {
                let value = args.get(i + 1).ok_or_else(|| {
                    io::Error::other("--cd-file needs a file path")
                })?;
                cd_file = Some(PathBuf::from(value));
                i += 1;
            }
            "-a" | "--hidden" => show_hidden = true,
            "--selftest" => selftest = true,
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(io::Error::other(format!(
                    "unknown option “{arg}” (try --help)"
                )));
            }
            _ => start = Some(arg.to_string()),
        }
        i += 1;
    }

    let (dir, select) = resolve_start(start.as_deref())?;

    if selftest {
        let mut app = App::new(dir, select, show_hidden);
        println!("{}", strip_escapes(&app.frame(100, 28)));
        return Ok(ExitCode::SUCCESS);
    }

    let mut console = Console::open()?;
    let outcome = App::new(dir, select, show_hidden).run(&mut console)?;
    drop(console);

    match outcome {
        Outcome::Jump(dir) => {
            let text = dir.to_string_lossy().into_owned();
            match &cd_file {
                Some(path) => fs::write(path, text.as_bytes())?,
                None => println!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Outcome::Cancel => Ok(ExitCode::from(1)),
    }
}

/// Work out where to open, and what to highlight once we are there.
fn resolve_start(arg: Option<&str>) -> io::Result<(Option<PathBuf>, Option<String>)> {
    let Some(arg) = arg else {
        return Ok((Some(std::env::current_dir()?), None));
    };

    let expanded = match arg.strip_prefix('~') {
        Some(rest) => {
            let home = std::env::var_os("USERPROFILE")
                .ok_or_else(|| io::Error::other("USERPROFILE is not set"))?;
            PathBuf::from(home).join(rest.trim_start_matches(['\\', '/']))
        }
        None => PathBuf::from(arg),
    };

    let absolute = tidy(std::path::absolute(&expanded)?);
    let meta = fs::metadata(&absolute)
        .map_err(|err| io::Error::new(err.kind(), format!("{}: {err}", absolute.display())))?;

    if meta.is_dir() {
        Ok((Some(absolute), None))
    } else {
        // Pointed at a file: open its folder with the file highlighted.
        let name = absolute.file_name().map(|n| n.to_string_lossy().into_owned());
        let parent = absolute.parent().map(Path::to_path_buf);
        Ok((parent, name))
    }
}

/// Drop the `\\?\` prefix that absolute paths sometimes carry, so the path we
/// show (and hand back to the shell) is the one people recognise.
fn tidy(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() > 2 && rest.as_bytes()[1] == b':' => PathBuf::from(rest),
        _ => path,
    }
}

/// Remove SGR/cursor sequences so `--selftest` output is readable.
fn strip_escapes(frame: &str) -> String {
    let mut out = String::with_capacity(frame.len());
    let mut chars = frame.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            while let Some(&next) = chars.peek() {
                chars.next();
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}
