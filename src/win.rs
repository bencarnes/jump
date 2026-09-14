//! The entire platform layer: a console we can draw to and read keys from.
//!
//! Output is done with VT sequences through `WriteConsoleW` (UTF-16, so the
//! console code page is irrelevant and emoji always survive). Input is read as
//! `INPUT_RECORD`s, which gives us arrow keys and Unicode characters without
//! having to parse escape sequences.

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_EXTENDED_FLAGS, ENABLE_PROCESSED_OUTPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT, GetConsoleMode,
    GetConsoleScreenBufferInfo, GetStdHandle, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode, WriteConsoleW,
};

// Virtual key codes and control-key flags. Declared here so we don't have to
// pull in extra `windows-sys` features beyond the ones microsoft/edit enables.
const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_C: u16 = 0x43;
const VK_H: u16 = 0x48;
const VK_R: u16 = 0x52;
const VK_F5: u16 = 0x74;

const RIGHT_ALT_PRESSED: u32 = 0x0001;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_CTRL_PRESSED: u32 = 0x0008;

/// A key press, already translated into something the UI cares about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Refresh,
    ToggleHidden,
    Cancel,
    Char(char),
}

pub struct Console {
    input: HANDLE,
    output: HANDLE,
    owned: bool,
    saved_input_mode: u32,
    saved_output_mode: u32,
    in_tui: bool,
    /// A pending UTF-16 high surrogate, waiting for its partner.
    surrogate: Option<u16>,
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn open_conhandle(name: &str) -> HANDLE {
    unsafe {
        CreateFileW(
            wide(name).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    }
}

impl Console {
    /// Attach to the console device itself (`CONIN$`/`CONOUT$`) so the UI keeps
    /// working even when stdin/stdout are redirected, and so stdout stays free
    /// for the directory we print on the way out.
    pub fn open() -> io::Result<Console> {
        let (input, output, owned) = {
            let i = open_conhandle("CONIN$");
            let o = open_conhandle("CONOUT$");
            if i == INVALID_HANDLE_VALUE || o == INVALID_HANDLE_VALUE {
                unsafe {
                    if i != INVALID_HANDLE_VALUE {
                        CloseHandle(i);
                    }
                    if o != INVALID_HANDLE_VALUE {
                        CloseHandle(o);
                    }
                    (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE), false)
                }
            } else {
                (i, o, true)
            }
        };

        let mut saved_input_mode = 0u32;
        let mut saved_output_mode = 0u32;
        let ok = unsafe {
            GetConsoleMode(input, &mut saved_input_mode) != 0
                && GetConsoleMode(output, &mut saved_output_mode) != 0
        };
        if !ok {
            return Err(io::Error::other(
                "not attached to a console (run jump from a terminal)",
            ));
        }

        Ok(Console {
            input,
            output,
            owned,
            saved_input_mode,
            saved_output_mode,
            in_tui: false,
            surrogate: None,
        })
    }

    /// Visible size of the window in cells.
    pub fn size(&self) -> (usize, usize) {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        if unsafe { GetConsoleScreenBufferInfo(self.output, &mut info) } == 0 {
            return (80, 25);
        }
        let w = (info.srWindow.Right - info.srWindow.Left + 1).max(1) as usize;
        let h = (info.srWindow.Bottom - info.srWindow.Top + 1).max(1) as usize;
        (w, h)
    }

    /// Switch to the alternate screen, turn off echo/line editing/auto-wrap.
    pub fn enter_tui(&mut self) -> io::Result<()> {
        unsafe {
            // No ENABLE_PROCESSED_INPUT: we want Ctrl+C as a key, not a signal.
            SetConsoleMode(self.input, ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS);
            SetConsoleMode(
                self.output,
                self.saved_output_mode
                    | ENABLE_PROCESSED_OUTPUT
                    | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            );
        }
        self.in_tui = true;
        // Alternate buffer, hide cursor, disable auto-wrap (so we may paint the
        // rightmost cell of every row without triggering a scroll).
        self.write_str("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J")
    }

    /// Undo `enter_tui`. Safe to call twice.
    pub fn leave_tui(&mut self) {
        if !self.in_tui {
            return;
        }
        self.in_tui = false;
        let _ = self.write_str("\x1b[?7h\x1b[?25h\x1b[0m\x1b[?1049l");
        unsafe {
            SetConsoleMode(self.input, self.saved_input_mode);
            SetConsoleMode(self.output, self.saved_output_mode);
        }
    }

    pub fn write_str(&self, s: &str) -> io::Result<()> {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let mut offset = 0usize;
        while offset < utf16.len() {
            let chunk = (utf16.len() - offset).min(16 * 1024);
            let mut written = 0u32;
            let ok = unsafe {
                WriteConsoleW(
                    self.output,
                    utf16[offset..].as_ptr(),
                    chunk as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if written == 0 {
                break;
            }
            offset += written as usize;
        }
        Ok(())
    }

    /// Block until at least one input event arrives, then translate the whole
    /// batch. An empty result means "nothing to do but repaint" (e.g. resize).
    pub fn read_keys(&mut self) -> io::Result<Vec<Key>> {
        let mut records: [INPUT_RECORD; 32] = unsafe { std::mem::zeroed() };
        let mut count = 0u32;
        let ok = unsafe {
            ReadConsoleInputW(self.input, records.as_mut_ptr(), records.len() as u32, &mut count)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut keys = Vec::new();
        for record in &records[..count as usize] {
            if record.EventType != KEY_EVENT as u16 {
                continue;
            }
            let ev = unsafe { &record.Event.KeyEvent };
            if ev.bKeyDown == 0 {
                continue;
            }
            let repeat = ev.wRepeatCount.clamp(1, 32);
            if let Some(key) = self.translate(ev.wVirtualKeyCode, unsafe { ev.uChar.UnicodeChar }, ev.dwControlKeyState) {
                for _ in 0..repeat {
                    keys.push(key);
                }
            }
        }
        Ok(keys)
    }

    fn translate(&mut self, vk: u16, unit: u16, control: u32) -> Option<Key> {
        let ctrl = control & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
        let alt = control & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0;

        // Named keys first: this also keeps Backspace (0x08) from colliding
        // with Ctrl+H, which reports the same character.
        match vk {
            VK_UP => return Some(Key::Up),
            VK_DOWN => return Some(Key::Down),
            VK_LEFT => return Some(Key::Left),
            VK_RIGHT => return Some(Key::Right),
            VK_RETURN => return Some(Key::Enter),
            VK_ESCAPE => return Some(Key::Escape),
            VK_BACK => return Some(Key::Backspace),
            VK_HOME => return Some(Key::Home),
            VK_END => return Some(Key::End),
            VK_PRIOR => return Some(Key::PageUp),
            VK_NEXT => return Some(Key::PageDown),
            VK_TAB => return Some(Key::Tab),
            VK_F5 => return Some(Key::Refresh),
            _ => {}
        }

        if ctrl && !alt {
            return match vk {
                VK_C => Some(Key::Cancel),
                VK_H => Some(Key::ToggleHidden),
                VK_R => Some(Key::Refresh),
                _ => None,
            };
        }
        if ctrl || alt {
            return None;
        }

        // Plain text, possibly arriving as a surrogate pair.
        if (0xD800..0xDC00).contains(&unit) {
            self.surrogate = Some(unit);
            return None;
        }
        let scalar = if (0xDC00..0xE000).contains(&unit) {
            let high = self.surrogate.take()? as u32;
            0x10000 + ((high - 0xD800) << 10) + (unit as u32 - 0xDC00)
        } else {
            self.surrogate = None;
            unit as u32
        };
        let c = char::from_u32(scalar)?;
        if c.is_control() { None } else { Some(Key::Char(c)) }
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.leave_tui();
        if self.owned {
            unsafe {
                CloseHandle(self.input);
                CloseHandle(self.output);
            }
        }
    }
}
