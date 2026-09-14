//! The browser itself: state, key handling and the frame renderer.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::entry::{Entry, Kind, format_size, list_dir, list_drives};
use crate::text;
use crate::win::{Console, Key};

/// How long a type-ahead search stays alive after the last keystroke.
const FIND_TIMEOUT: Duration = Duration::from_millis(1500);

pub enum Outcome {
    /// Quit and take the shell to this directory.
    Jump(PathBuf),
    /// Quit and leave the shell where it was.
    Cancel,
}

pub struct App {
    /// The directory being browsed, or `None` for the drive list ("This PC").
    cwd: Option<PathBuf>,
    entries: Vec<Entry>,
    sel: usize,
    scroll: usize,
    show_hidden: bool,
    find: String,
    find_stamp: Instant,
    status: Option<String>,
    done: Option<Outcome>,
}

impl App {
    pub fn new(start: Option<PathBuf>, select: Option<String>, show_hidden: bool) -> App {
        let mut app = App {
            cwd: None,
            entries: Vec::new(),
            sel: 0,
            scroll: 0,
            show_hidden,
            find: String::new(),
            find_stamp: Instant::now(),
            status: None,
            done: None,
        };
        app.goto(start, select);
        // A start directory we cannot read should not leave us with nothing.
        if app.entries.is_empty() {
            let complaint = app.status.take();
            app.goto(None, None);
            app.status = complaint;
        }
        app
    }

    pub fn run(&mut self, console: &mut Console) -> io::Result<Outcome> {
        console.enter_tui()?;
        loop {
            let (w, h) = console.size();
            let frame = self.frame(w, h);
            console.write_str(&frame)?;

            for key in console.read_keys()? {
                self.handle(key);
                if self.done.is_some() {
                    break;
                }
            }
            if let Some(outcome) = self.done.take() {
                console.leave_tui();
                return Ok(outcome);
            }
        }
    }

    /// Render one screen, ready to be written in a single call.
    pub fn frame(&mut self, width: usize, height: usize) -> String {
        let w = width.max(24);
        let h = height.max(6);
        let rows = h - 4;
        self.clamp(rows);

        let mut out = String::with_capacity(w * h * 2);
        out.push_str("\x1b[H");

        // Header: where we are.
        let title = match &self.cwd {
            Some(dir) => format!("📍 {}", dir.display()),
            None => "💻 This PC".to_string(),
        };
        out.push_str("\x1b[1m");
        out.push_str(&text::fit(&format!(" {}", text::truncate_left(&title, w - 2)), w));
        out.push_str("\x1b[0m\r\n");

        // Rule.
        out.push_str("\x1b[2m");
        out.push_str(&"─".repeat(w));
        out.push_str("\x1b[0m\r\n");

        // Entries.
        for row in 0..rows {
            match self.entries.get(self.scroll + row) {
                Some(entry) => out.push_str(&self.row(entry, self.scroll + row == self.sel, w)),
                None => out.push_str(&" ".repeat(w)),
            }
            out.push_str("\r\n");
        }

        // Rule.
        out.push_str("\x1b[2m");
        out.push_str(&"─".repeat(w));
        out.push_str("\x1b[0m\r\n");

        // Footer: status, or the live search, or the key map.
        let left = if let Some(status) = &self.status {
            format!(" ⚠ {status}")
        } else if !self.find.is_empty() {
            format!(" find: {}▏ (Tab next · Esc clear)", self.find)
        } else {
            " ↑↓ move   →/Enter open   ←/Bksp up   Esc jump here   Ctrl+C cancel".to_string()
        };
        let mut right = if self.entries.is_empty() {
            "empty".to_string()
        } else {
            format!("{}/{}", self.sel + 1, self.entries.len())
        };
        if self.show_hidden {
            right.push_str(" · hidden");
        }
        right.push(' ');

        let right_w = text::width(&right);
        out.push_str("\x1b[2m");
        out.push_str(&text::fit(&left, w.saturating_sub(right_w)));
        out.push_str(&right);
        out.push_str("\x1b[0m");
        out
    }

    fn row(&self, entry: &Entry, selected: bool, w: usize) -> String {
        let size = match entry.kind {
            Kind::File => format_size(entry.size),
            _ => String::new(),
        };
        let size_w = text::width(&size);
        let gap = if size_w == 0 { 0 } else { 2 };
        let label_w = (w - 2).saturating_sub(size_w + gap);
        let label = text::fit(&format!("{} {}", entry.icon, entry.name), label_w);

        let body = format!(" {label}{}{size} ", " ".repeat(gap));
        let body = text::fit(&body, w);

        let color = match entry.kind {
            _ if selected => "\x1b[7m",
            Kind::Parent => "\x1b[2m",
            Kind::Dir => "\x1b[94m",
            Kind::Drive => "\x1b[96m",
            Kind::File => "",
        };
        if color.is_empty() { body } else { format!("{color}{body}\x1b[0m") }
    }

    fn clamp(&mut self, rows: usize) {
        if self.entries.is_empty() {
            self.sel = 0;
            self.scroll = 0;
            return;
        }
        self.sel = self.sel.min(self.entries.len() - 1);
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if self.sel >= self.scroll + rows {
            self.scroll = self.sel + 1 - rows;
        }
        let max_scroll = self.entries.len().saturating_sub(rows);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn handle(&mut self, key: Key) {
        self.status = None;
        if self.find_stamp.elapsed() > FIND_TIMEOUT {
            self.find.clear();
        }

        match key {
            Key::Up => {
                self.find.clear();
                self.sel = self.sel.saturating_sub(1);
            }
            Key::Down => {
                self.find.clear();
                if self.sel + 1 < self.entries.len() {
                    self.sel += 1;
                }
            }
            Key::PageUp => {
                self.find.clear();
                self.sel = self.sel.saturating_sub(10);
            }
            Key::PageDown => {
                self.find.clear();
                self.sel = (self.sel + 10).min(self.entries.len().saturating_sub(1));
            }
            Key::Home => {
                self.find.clear();
                self.sel = 0;
            }
            Key::End => {
                self.find.clear();
                self.sel = self.entries.len().saturating_sub(1);
            }
            Key::Left => self.go_up(),
            Key::Backspace => {
                // While typing, Backspace edits the search; otherwise it walks up.
                if self.find.pop().is_some() {
                    self.find_stamp = Instant::now();
                    let find = self.find.clone();
                    if !find.is_empty() && let Some(i) = self.match_from(&find, 0) {
                        self.sel = i;
                    }
                } else {
                    self.go_up();
                }
            }
            Key::Right | Key::Enter => self.open(),
            Key::Tab => {
                if self.find.is_empty() {
                    return;
                }
                self.find_stamp = Instant::now();
                let find = self.find.clone();
                if let Some(i) = self.match_from(&find, self.sel + 1) {
                    self.sel = i;
                }
            }
            Key::Refresh => self.reload(),
            Key::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.reload();
            }
            Key::Escape => {
                if self.find.is_empty() {
                    self.done = Some(match self.jump_target() {
                        Some(dir) => Outcome::Jump(dir),
                        None => Outcome::Cancel,
                    });
                } else {
                    self.find.clear();
                }
            }
            Key::Cancel => self.done = Some(Outcome::Cancel),
            Key::Char(c) => self.type_ahead(c),
        }
    }

    /// Explorer-style type-to-select: keep appending while the user types, and
    /// ignore a keystroke that would match nothing.
    fn type_ahead(&mut self, c: char) {
        let mut candidate = self.find.clone();
        candidate.push(c);
        match self.match_from(&candidate, 0) {
            Some(i) => {
                self.sel = i;
                self.find = candidate;
                self.find_stamp = Instant::now();
            }
            None => {
                // A single unmatched letter is more likely a fresh search than a typo.
                if !self.find.is_empty()
                    && let Some(i) = self.match_from(&c.to_string(), 0)
                {
                    self.sel = i;
                    self.find = c.to_string();
                    self.find_stamp = Instant::now();
                    return;
                }
                self.status = Some(format!("no match for “{candidate}”"));
            }
        }
    }

    /// First entry matching `needle`, searching from `start` and wrapping.
    /// Name prefixes win; a substring match is the fallback.
    fn match_from(&self, needle: &str, start: usize) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let needle = needle.to_lowercase();
        let order = (start..self.entries.len()).chain(0..start.min(self.entries.len()));
        let names: Vec<String> = self.entries.iter().map(|e| e.name.to_lowercase()).collect();
        let order: Vec<usize> = order.collect();
        order
            .iter()
            .copied()
            .find(|&i| names[i].starts_with(&needle))
            .or_else(|| order.iter().copied().find(|&i| names[i].contains(&needle)))
    }

    fn open(&mut self) {
        let Some(entry) = self.entries.get(self.sel) else { return };
        match entry.kind {
            Kind::Parent => self.go_up(),
            Kind::Dir | Kind::Drive => {
                let target = entry.path.clone();
                self.goto(Some(target), None);
            }
            // Confirming a file means "this is the folder I wanted".
            Kind::File => {
                self.done = Some(match self.jump_target() {
                    Some(dir) => Outcome::Jump(dir),
                    None => Outcome::Cancel,
                });
            }
        }
    }

    fn go_up(&mut self) {
        let Some(dir) = self.cwd.clone() else {
            self.status = Some("this is the top — Esc to jump, Ctrl+C to cancel".into());
            return;
        };
        let leaf = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => {
                let parent = parent.to_path_buf();
                self.goto(Some(parent), leaf);
            }
            // Already at a drive root: step out to the drive list.
            _ => {
                let root = dir.to_string_lossy().into_owned();
                self.goto(None, Some(root));
            }
        }
    }

    fn reload(&mut self) {
        let here = self.cwd.clone();
        let selected = self.entries.get(self.sel).map(|e| e.name.clone());
        self.goto(here, selected);
    }

    /// Move to `target` (`None` = drive list), selecting `select` if present.
    /// A directory we cannot read leaves the view untouched.
    fn goto(&mut self, target: Option<PathBuf>, select: Option<String>) {
        let entries = match &target {
            None => list_drives(),
            Some(dir) => match list_dir(dir, self.show_hidden) {
                Ok(children) => {
                    let mut all = Vec::with_capacity(children.len() + 1);
                    all.push(parent_row(dir));
                    all.extend(children);
                    all
                }
                Err(err) => {
                    self.status = Some(format!("{}: {}", dir.display(), reason(&err)));
                    return;
                }
            },
        };

        self.cwd = target;
        self.entries = entries;
        self.find.clear();
        self.scroll = 0;
        self.sel = select
            .and_then(|name| {
                let name = name.to_lowercase();
                self.entries
                    .iter()
                    .position(|e| e.name.to_lowercase() == name)
                    .or_else(|| {
                        self.entries.iter().position(|e| e.name.to_lowercase().starts_with(&name))
                    })
            })
            .unwrap_or(0);
    }

    fn jump_target(&self) -> Option<PathBuf> {
        match &self.cwd {
            Some(dir) => Some(dir.clone()),
            // In the drive list, the highlighted drive is the destination.
            None => self.entries.get(self.sel).map(|e| e.path.clone()),
        }
    }
}

fn parent_row(dir: &Path) -> Entry {
    Entry {
        name: "..".to_string(),
        icon: "📂",
        kind: Kind::Parent,
        path: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
        size: 0,
    }
}

fn reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::PermissionDenied => "access denied".to_string(),
        io::ErrorKind::NotFound => "not found".to_string(),
        _ => err.to_string(),
    }
}
