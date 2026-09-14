//! Directory listings: what we show, how it is labelled, how it is ordered.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The synthetic `..` row.
    Parent,
    Dir,
    File,
    Drive,
}

pub struct Entry {
    /// What the user sees and types to select the row.
    pub name: String,
    pub icon: &'static str,
    pub kind: Kind,
    /// Where activating the row takes us (for files: the file itself).
    pub path: PathBuf,
    pub size: u64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        !matches!(self.kind, Kind::File)
    }
}

/// List a directory, sorted folders-first and then case-insensitively by name.
pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();

        let meta = item.metadata().ok();
        let attrs = meta.as_ref().map(|m| m.file_attributes()).unwrap_or(0);
        let concealed = attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
            || name.starts_with('.');
        if concealed && !show_hidden {
            continue;
        }

        let file_type = item.file_type().ok();
        let link = file_type.as_ref().is_some_and(|t| t.is_symlink());
        // Symlinks and junctions report their own type, so resolve those to see
        // whether we can descend into them.
        let is_dir = match &file_type {
            Some(t) if t.is_dir() => true,
            Some(t) if t.is_symlink() => fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false),
            _ => false,
        };

        entries.push(Entry {
            icon: if link {
                "🔗"
            } else if is_dir {
                "📁"
            } else {
                icon_for_file(&name)
            },
            name,
            kind: if is_dir { Kind::Dir } else { Kind::File },
            path,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir()
            .cmp(&a.is_dir())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// The "This PC" view: one row per logical drive.
pub fn list_drives() -> Vec<Entry> {
    let mask = unsafe { GetLogicalDrives() };
    let mut entries = Vec::new();
    for bit in 0..26u32 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        let letter = (b'A' + bit as u8) as char;
        let root = format!("{letter}:\\");
        let kind = unsafe { GetDriveTypeW(wide(&root).as_ptr()) };
        let icon = match kind {
            DRIVE_REMOVABLE => "🔌",
            DRIVE_REMOTE => "🌐",
            DRIVE_CDROM => "💿",
            DRIVE_RAMDISK => "⚡",
            _ => "💽",
        };
        // Only fixed drives get a label lookup: querying a disconnected
        // network share or an empty optical drive can stall for seconds.
        let name = match (kind == DRIVE_FIXED).then(|| volume_label(&root)).flatten() {
            Some(label) => format!("{letter}:\\   {label}"),
            None => root.clone(),
        };
        entries.push(Entry {
            name,
            icon,
            kind: Kind::Drive,
            path: PathBuf::from(root),
            size: 0,
        });
    }
    entries
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn volume_label(root: &str) -> Option<String> {
    let mut buf = [0u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(
            wide(root).as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let label = std::ffi::OsString::from_wide(&buf[..len]).to_string_lossy().into_owned();
    if label.is_empty() { None } else { Some(label) }
}

/// A rough emoji for a file, picked from its extension.
pub fn icon_for_file(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    let ext = Path::new(&lower).extension().and_then(OsStr::to_str).unwrap_or("");

    // A few names are more telling than their extension.
    match lower.as_str() {
        "cargo.toml" | "cargo.lock" => return "🦀",
        "makefile" | "dockerfile" | "cmakelists.txt" => return "🔨",
        "license" | "license.md" | "license.txt" | "copying" => return "⚖",
        "readme" | "readme.md" | "readme.txt" => return "📖",
        _ => {}
    }

    match ext {
        "rs" => "🦀",
        "py" | "pyw" | "ipynb" => "🐍",
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" => "📜",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "cs" | "java" | "kt" | "go" | "rb"
        | "php" | "swift" | "scala" | "lua" | "pl" | "r" | "zig" | "dart" | "ex" | "erl"
        | "hs" | "ml" | "fs" | "vb" | "asm" | "s" => "💻",
        "ps1" | "psm1" | "psd1" | "sh" | "bash" | "zsh" | "bat" | "cmd" => "🐚",
        "html" | "htm" | "css" | "scss" | "sass" | "less" | "xml" | "xaml" | "svg" | "vue" => {
            "🌐"
        }
        "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "env" | "properties"
        | "editorconfig" | "gitignore" | "gitattributes" => "🔧",
        "md" | "markdown" | "rst" | "adoc" | "org" => "📝",
        "txt" | "text" | "nfo" => "📃",
        "log" | "out" | "err" => "🧾",
        "pdf" => "📕",
        "doc" | "docx" | "odt" | "rtf" | "pages" => "📘",
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" | "numbers" => "📊",
        "ppt" | "pptx" | "odp" | "key" => "📙",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff" | "psd" | "ai"
        | "heic" | "avif" | "raw" | "cr2" => "🎨",
        "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "mid" | "midi" => "🎵",
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "flv" | "m4v" | "mpg" | "mpeg" => "🎬",
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" | "xz" | "zst" | "bz2" | "cab" | "iso"
        | "vhd" | "vhdx" | "img" | "whl" | "jar" | "nupkg" | "msix" => "📦",
        "exe" | "msi" | "dll" | "so" | "dylib" | "com" | "sys" | "appx" => "⚙",
        "ttf" | "otf" | "woff" | "woff2" | "fon" => "🔤",
        "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "parquet" | "bak" => "💾",
        "pem" | "crt" | "cer" | "pfx" | "p12" | "pub" | "asc" | "gpg" => "🔑",
        "lock" => "🔒",
        "o" | "obj" | "pdb" | "lib" | "a" | "class" | "pyc" | "rlib" | "d" => "🧱",
        "ttc" | "lnk" | "url" | "desktop" => "🔗",
        "" => "📄",
        _ => "📄",
    }
}

/// Human-readable file size, always short enough for the right-hand column.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_stay_short() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(20 * 1024), "20 KB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
        for bytes in [0, 1, 999, 1 << 20, 1 << 40, u64::MAX] {
            assert!(format_size(bytes).len() <= 8, "{bytes}");
        }
    }

    #[test]
    fn icons_cover_the_common_cases() {
        assert_eq!(icon_for_file("main.rs"), "🦀");
        assert_eq!(icon_for_file("Cargo.toml"), "🦀"); // name beats extension
        assert_eq!(icon_for_file("notes.MD"), "📝"); // case-insensitive
        assert_eq!(icon_for_file("report.docx"), "📘");
        assert_eq!(icon_for_file("budget.xlsx"), "📊");
        assert_eq!(icon_for_file("photo.jpeg"), "🎨");
        assert_eq!(icon_for_file("no-extension"), "📄");
        assert_eq!(icon_for_file(".hidden"), "📄");
    }

    #[test]
    fn drives_are_roots_we_can_navigate_to() {
        let drives = list_drives();
        assert!(!drives.is_empty(), "a Windows box always has a drive");
        for drive in &drives {
            assert_eq!(drive.kind, Kind::Drive);
            let text = drive.path.to_string_lossy();
            assert!(text.len() == 3 && text.ends_with(":\\"), "{text}");
        }
    }

    #[test]
    fn listing_is_folders_first_then_case_insensitive() {
        let entries = list_dir(Path::new("."), true).expect("cwd is readable");
        let boundary = entries.iter().position(|e| !e.is_dir()).unwrap_or(entries.len());
        assert!(entries[boundary..].iter().all(|e| !e.is_dir()));
        for pair in entries.windows(2) {
            if pair[0].is_dir() == pair[1].is_dir() {
                assert!(
                    pair[0].name.to_lowercase() <= pair[1].name.to_lowercase(),
                    "{} before {}",
                    pair[0].name,
                    pair[1].name
                );
            }
        }
    }
}
