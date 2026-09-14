//! Shell integration.
//!
//! A child process cannot change its parent's working directory, so `jump`
//! writes the directory you picked into a file and a tiny shell function does
//! the `cd`. `jump --install` adds that function to your PowerShell profile.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const BEGIN: &str = "# >>> jump shell integration >>>";
const END: &str = "# <<< jump shell integration <<<";

pub fn print_init(shell: &str) -> io::Result<()> {
    let exe = exe_path()?;
    let text = match shell.to_lowercase().as_str() {
        "powershell" | "pwsh" | "ps" | "ps1" => powershell_snippet(&exe),
        "cmd" | "bat" | "batch" => cmd_snippet(&exe),
        "bash" | "sh" | "zsh" | "git-bash" => bash_snippet(&exe),
        other => {
            return Err(io::Error::other(format!(
                "unknown shell “{other}” (try powershell, cmd or bash)"
            )));
        }
    };
    println!("{text}");
    Ok(())
}

/// Add (or refresh) the integration block in every PowerShell profile we find.
pub fn install() -> io::Result<()> {
    let exe = exe_path()?;
    let snippet = powershell_snippet(&exe);
    let profiles = powershell_profiles();

    if profiles.is_empty() {
        return Err(io::Error::other(
            "could not locate a PowerShell profile; run `jump --init powershell` and paste the \
             output into your profile by hand",
        ));
    }

    println!("jump: using {}", exe.display());
    for profile in profiles {
        match write_block(&profile, Some(&snippet)) {
            Ok(action) => println!("  {action} {}", profile.display()),
            Err(err) => println!("  failed {}: {err}", profile.display()),
        }
    }
    println!();
    println!("Open a new shell (or run `. $PROFILE`) and type `jump`.");
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let profiles = powershell_profiles();
    if profiles.is_empty() {
        return Err(io::Error::other("could not locate a PowerShell profile"));
    }
    for profile in profiles {
        match write_block(&profile, None) {
            Ok(action) => println!("  {action} {}", profile.display()),
            Err(err) => println!("  failed {}: {err}", profile.display()),
        }
    }
    Ok(())
}

fn exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Ask each installed PowerShell where its per-user profile lives, rather than
/// guessing at Documents\ (which OneDrive likes to move).
fn powershell_profiles() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for shell in ["pwsh", "powershell"] {
        if let Some(path) = ask_profile_path(shell)
            && !found.contains(&path)
        {
            found.push(path);
        }
    }
    found
}

fn ask_profile_path(shell: &str) -> Option<PathBuf> {
    let scratch = temp_file("jump-profile");
    let escaped = scratch.display().to_string().replace('\'', "''");
    let script = format!(
        "[IO.File]::WriteAllText('{escaped}', $PROFILE.CurrentUserCurrentHost, \
         (New-Object System.Text.UTF8Encoding $false))"
    );
    let status = Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-NoLogo", "-Command", &script])
        .status();

    let result = match status {
        Ok(status) if status.success() => fs::read_to_string(&scratch)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
        _ => None,
    };
    let _ = fs::remove_file(&scratch);
    result
}

fn temp_file(stem: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{stem}-{}-{nonce}.txt", std::process::id()))
}

/// Insert, replace or remove the marked block in `path`.
fn write_block(path: &Path, block: Option<&str>) -> io::Result<&'static str> {
    let raw = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err),
    };
    let bom = raw.starts_with(&[0xEF, 0xBB, 0xBF]);
    let body = if bom { &raw[3..] } else { &raw[..] };
    let existing = String::from_utf8(body.to_vec()).map_err(|_| {
        io::Error::other("profile is not UTF-8; add the snippet manually (jump --init powershell)")
    })?;

    let region = existing.find(BEGIN).and_then(|start| {
        existing[start..].find(END).map(|len| start..start + len + END.len())
    });

    let (updated, action) = match (region, block) {
        (Some(region), Some(block)) => {
            let mut next = existing.clone();
            next.replace_range(region, block);
            (next, "updated")
        }
        (Some(region), None) => {
            let mut next = existing.clone();
            next.replace_range(region, "");
            (next.trim_end().to_string() + "\r\n", "removed from")
        }
        (None, Some(block)) => {
            let mut next = existing.trim_end().to_string();
            if !next.is_empty() {
                next.push_str("\r\n\r\n");
            }
            next.push_str(block);
            next.push_str("\r\n");
            (next, "added to")
        }
        (None, None) => return Ok("nothing to remove in"),
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out: Vec<u8> = Vec::new();
    if bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(updated.as_bytes());
    fs::write(path, out)?;
    Ok(action)
}

fn powershell_snippet(exe: &Path) -> String {
    let exe = exe.display().to_string().replace('\'', "''");
    format!(
        "{BEGIN}
function jump {{
    $jumpExe = '{exe}'
    $cdFile = Join-Path ([IO.Path]::GetTempPath()) (\"jump-cd-\" + [guid]::NewGuid().ToString('N'))
    try {{
        & $jumpExe --cd-file $cdFile @args
        if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $cdFile)) {{
            $dest = (Get-Content -LiteralPath $cdFile -Raw -Encoding UTF8)
            if ($dest) {{ $dest = $dest.Trim().TrimStart([char]0xFEFF) }}
            if ($dest -and (Test-Path -LiteralPath $dest -PathType Container)) {{
                Set-Location -LiteralPath $dest
            }}
        }}
    }} finally {{
        Remove-Item -LiteralPath $cdFile -Force -ErrorAction SilentlyContinue
    }}
}}
if (-not (Get-Command j -ErrorAction SilentlyContinue)) {{ Set-Alias -Name j -Value jump }}
{END}"
    )
}

fn cmd_snippet(exe: &Path) -> String {
    let exe = exe.display();
    format!(
        "@echo off
rem Save this as jump.cmd somewhere on your PATH, then run `jump`.
set \"JUMP_CD_FILE=%TEMP%\\jump-cd-%RANDOM%%RANDOM%.txt\"
\"{exe}\" --cd-file \"%JUMP_CD_FILE%\" %*
set \"JUMP_DEST=\"
if exist \"%JUMP_CD_FILE%\" set /p JUMP_DEST=<\"%JUMP_CD_FILE%\"
if exist \"%JUMP_CD_FILE%\" del \"%JUMP_CD_FILE%\" >nul 2>&1
if defined JUMP_DEST cd /d \"%JUMP_DEST%\"
set \"JUMP_DEST=\"
set \"JUMP_CD_FILE=\""
    )
}

fn bash_snippet(exe: &Path) -> String {
    let exe = exe.display().to_string().replace('\\', "/");
    format!(
        "# Add to ~/.bashrc:  eval \"$(/path/to/jump.exe --init bash)\"
jump() {{
    local cd_file dest code
    cd_file=\"$(mktemp)\"
    if command -v cygpath >/dev/null 2>&1; then
        \"{exe}\" --cd-file \"$(cygpath -w \"$cd_file\")\" \"$@\"
    else
        \"{exe}\" --cd-file \"$cd_file\" \"$@\"
    fi
    code=$?
    dest=\"\"
    [ -s \"$cd_file\" ] && dest=\"$(cat \"$cd_file\")\"
    rm -f \"$cd_file\"
    if [ \"$code\" -eq 0 ] && [ -n \"$dest\" ]; then
        if command -v cygpath >/dev/null 2>&1; then dest=\"$(cygpath -u \"$dest\")\"; fi
        cd \"$dest\" || return 1
    fi
}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "# >>> jump shell integration >>>\nfunction jump { 'v1' }\n# <<< jump shell integration <<<";
    const V2: &str = "# >>> jump shell integration >>>\nfunction jump { 'v2' }\n# <<< jump shell integration <<<";

    fn scratch(name: &str) -> PathBuf {
        let path = temp_file(&format!("jump-test-{name}"));
        let _ = fs::remove_file(&path);
        path
    }

    #[test]
    fn block_is_added_updated_and_removed_in_place() {
        let profile = scratch("profile");
        fs::write(&profile, "Set-Alias ll Get-ChildItem\r\n").unwrap();

        assert_eq!(write_block(&profile, Some(V1)).unwrap(), "added to");
        let after_add = fs::read_to_string(&profile).unwrap();
        assert!(after_add.starts_with("Set-Alias ll Get-ChildItem"), "existing profile survives");
        assert!(after_add.contains("'v1'"));

        assert_eq!(write_block(&profile, Some(V2)).unwrap(), "updated");
        let after_update = fs::read_to_string(&profile).unwrap();
        assert_eq!(after_update.matches(BEGIN).count(), 1, "no duplicate blocks");
        assert!(after_update.contains("'v2'") && !after_update.contains("'v1'"));
        assert!(after_update.contains("Set-Alias ll Get-ChildItem"));

        assert_eq!(write_block(&profile, None).unwrap(), "removed from");
        let after_remove = fs::read_to_string(&profile).unwrap();
        assert!(!after_remove.contains(BEGIN) && !after_remove.contains(END));
        assert!(after_remove.contains("Set-Alias ll Get-ChildItem"));

        assert_eq!(write_block(&profile, None).unwrap(), "nothing to remove in");
        fs::remove_file(&profile).unwrap();
    }

    #[test]
    fn a_missing_profile_is_created_and_a_bom_is_preserved() {
        let fresh = scratch("fresh");
        assert_eq!(write_block(&fresh, Some(V1)).unwrap(), "added to");
        assert!(fs::read_to_string(&fresh).unwrap().contains(BEGIN));
        fs::remove_file(&fresh).unwrap();

        let bom = scratch("bom");
        let mut original = vec![0xEF, 0xBB, 0xBF];
        original.extend_from_slice("# caf\u{e9}\r\n".as_bytes());
        fs::write(&bom, &original).unwrap();
        write_block(&bom, Some(V1)).unwrap();
        let bytes = fs::read(&bom).unwrap();
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM kept");
        assert!(String::from_utf8(bytes).unwrap().contains("caf\u{e9}"));
        fs::remove_file(&bom).unwrap();
    }

    #[test]
    fn snippets_embed_the_executable_path() {
        let exe = Path::new(r"C:\tools\jump.exe");
        for text in [powershell_snippet(exe), cmd_snippet(exe), bash_snippet(exe)] {
            assert!(text.contains("jump.exe"), "{text}");
            assert!(text.contains("--cd-file"), "{text}");
        }
        assert!(powershell_snippet(exe).contains(BEGIN));
        assert!(bash_snippet(exe).contains("C:/tools/jump.exe"), "bash wants forward slashes");
    }
}
