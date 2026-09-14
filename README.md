# jump

A small terminal app for walking the filesystem with the arrow keys, and for
leaving your shell in the folder you picked.

```
 📍 C:\SourcePersonalTools\jump
────────────────────────────────────────────────────────────────────
 📂 ..
 📁 src
 📁 target
 🦀 Cargo.lock                                                611 B
 🦀 Cargo.toml                                                650 B
 📝 README.md                                                4.2 KB
────────────────────────────────────────────────────────────────────
 ↑↓ move   →/Enter open   ←/Bksp up   Esc jump here   Ctrl+C cancel
```

## Keys

| Key | What it does |
| --- | --- |
| `↑` `↓` | move the selection |
| `PgUp` `PgDn` | move by ten |
| `Home` `End` | first / last entry |
| `→` or `Enter` | open the highlighted folder |
| `←` or `Backspace` | go up a folder (`Backspace` edits an active search first) |
| *type a name* | select it, the way Explorer does |
| `Tab` | next match for what you typed |
| `Esc` | quit, and take the shell to the folder you are in |
| `Ctrl+C` | quit and leave the shell where it was |
| `Ctrl+H` | show / hide hidden and system files |
| `F5` / `Ctrl+R` | re-read the folder |

Going up from a drive root (`C:\`) lands you in a drive list; picking a drive
there and pressing `Esc` jumps to that drive.

Typing is a live prefix search — the footer shows what you have typed and the
buffer clears itself after about 1.5 seconds of silence. Names win on prefix
first and fall back to a substring match, so `car` finds `Cargo.toml` and `dow`
finds `Downloads`.

## Build

```powershell
cargo build --release
```

The binary lands in `target\release\jump.exe`.

## Making the shell follow you (`cd` on exit)

A process cannot change its parent's working directory, so `jump` does the only
thing that works: it writes the folder you chose into a file and a three-line
shell function runs the `cd`.

Install the function into your PowerShell profile:

```powershell
.\target\release\jump.exe --install
```

That appends a marked block to `$PROFILE` (the real one, asked of PowerShell
itself — it is often under OneDrive), embedding the full path of the exe you
ran it from. Re-running `--install` after moving or rebuilding the binary
rewrites the block instead of duplicating it, and `--uninstall` takes it back
out. Open a new shell, or `. $PROFILE`, and then:

```powershell
jump          # or `j`, aliased only if nothing else owns that name
jump C:\Windows
jump ~\Downloads
```

Prefer to see the snippet before it touches anything, or use another shell:

```powershell
jump --init powershell   # what --install writes
jump --init cmd          # save as jump.cmd on your PATH
jump --init bash         # for Git Bash: eval "$(jump.exe --init bash)" in ~/.bashrc
```

Without any of that, `jump` still works — it just prints the chosen directory
to stdout on exit, so `Set-Location (jump)` does the job by hand.

### How the handshake works

- The UI is drawn to the console device (`CONOUT$`) rather than to stdout, so
  stdout stays clean and redirection never breaks the display.
- `--cd-file <path>` writes the chosen directory there; the wrapper reads it,
  deletes it and calls `Set-Location`.
- Exit code `0` means "jump here", `1` means "cancelled, stay put", `2` is an
  error. The wrapper only moves on `0`.

## Options

```
    -a, --hidden       start with hidden files shown
        --cd-file <F>  write the chosen directory to <F> (used by the wrapper)
        --init <SHELL> print the wrapper for powershell, cmd or bash
        --install      add the wrapper to your PowerShell profile
        --uninstall    remove it again
        --selftest     render one frame to stdout and exit, no TUI (diagnostics)
    -h, --help         help
    -V, --version      version
```

`PATH` can point straight at `jump.exe`, but note the shell function shadows
the exe by name on purpose — that is what makes the directory change stick.

## Dependencies

Only the ones [microsoft/edit](https://github.com/microsoft/edit) uses:
`windows-sys` on Windows (and `libc` on Unix, once a Unix backend exists). The
UI, input handling, text measuring and layout are all hand-rolled:

| File | Role |
| --- | --- |
| `src/win.rs` | console handles, raw mode, `ReadConsoleInputW`, `WriteConsoleW` |
| `src/app.rs` | state, key handling, the frame renderer |
| `src/entry.rs` | directory and drive listings, emoji per file type, sizes |
| `src/text.rs` | terminal cell widths, truncation, padding |
| `src/shell.rs` | the wrapper snippets and profile installer |

Output is VT escape sequences on the alternate screen buffer; input arrives as
console input records, so arrow keys and Unicode both work without parsing
escape sequences. `cargo test` covers the listing order, size and icon
formatting, cell-width maths and the profile editor.

Windows only for now: `src/win.rs` is the whole platform layer, and a Unix
version of it (termios + VT input parsing via `libc`) has not been written.
