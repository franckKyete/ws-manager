/// Cross-platform clipboard helper supporting persistent native clipboard, Linux CLI utilities, and ANSI OSC 52.

use std::io::{stdout, Write};
use std::sync::Mutex;
use std::sync::OnceLock;
use base64::Engine;

static CLIPBOARD: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();

fn get_clipboard() -> &'static Mutex<Option<arboard::Clipboard>> {
    CLIPBOARD.get_or_init(|| {
        let cb = arboard::Clipboard::new().ok();
        Mutex::new(cb)
    })
}

pub fn copy_to_clipboard(text: &str) {
    if text.is_empty() {
        return;
    }

    // 1. Persistent native system clipboard via arboard (keeps clipboard alive without drop warnings)
    let cb_mutex = get_clipboard();
    if let Ok(mut guard) = cb_mutex.lock() {
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
        if let Some(ref mut cb) = *guard {
            let _ = cb.set_text(text.to_string());
        }
    }

    // 2. Fallback CLI utilities on Linux (wl-copy, xclip) if available
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            let _ = std::process::Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .and_then(|mut child| {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    child.wait()
                });
        } else if std::env::var("DISPLAY").is_ok() {
            let _ = std::process::Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .and_then(|mut child| {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    child.wait()
                });
        }
    }

    // 3. Universal ANSI OSC 52 escape sequence (works over SSH, tmux, and modern terminal emulators)
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let osc52 = format!("\x1b]52;c;{}\x07", encoded);
    let mut out = stdout();
    let _ = out.write_all(osc52.as_bytes());
    let _ = out.flush();
}

pub fn get_from_clipboard() -> Option<String> {
    let cb_mutex = get_clipboard();
    if let Ok(mut guard) = cb_mutex.lock() {
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
        if let Some(ref mut cb) = *guard {
            if let Ok(text) = cb.get_text() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }

    // Fallback CLI utility on Linux
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            if let Ok(output) = std::process::Command::new("wl-paste").output() {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }

    None
}
