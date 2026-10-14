use std::ffi::CStr;
use std::io::Write;
use std::os::raw::c_char;
use std::process::{Command, Stdio};
use std::thread;

/// Internal function to copy text to clipboard and trigger notification in background thread
pub fn copy_and_notify_async(text: String) {
    if text.is_empty() {
        return;
    }

    thread::spawn(move || {
        let is_wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.to_lowercase() == "wayland")
            .unwrap_or(false);

        let mut copied = false;

        // Try Wayland wl-copy first if Wayland or if wl-copy is present
        if is_wayland {
            if let Ok(mut child) = Command::new("wl-copy")
                .arg("-n")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                copied = true;
            }
        }

        // Fallback to xclip (X11) if not copied
        if !copied {
            if let Ok(mut child) = Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                copied = true;
            }
        }

        // Fallback to wl-copy even if session type was not explicitly detected
        if !copied {
            if let Ok(mut child) = Command::new("wl-copy")
                .arg("-n")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }

        // Fire notification asynchronously (fire and forget, zero GUI wait)
        let lines = text.lines().count();
        let title = "Đã sao chép đường dẫn";
        let body = if lines <= 1 {
            text.clone()
        } else {
            format!("Đã sao chép {} đường dẫn vào clipboard", lines)
        };

        let _ = Command::new("notify-send")
            .args(["-a", "Files", "-i", "edit-copy", title, &body, "-t", "2000"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    });
}

/// C ABI exported function for dynamic linking (ctypes / FFI)
/// Returns 1 on success, 0 on null pointer or error
#[no_mangle]
pub extern "C" fn copy_path(path_ptr: *const c_char) -> i32 {
    if path_ptr.is_null() {
        return 0;
    }

    let c_str = unsafe { CStr::from_ptr(path_ptr) };
    let text = match c_str.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return 0,
    };

    copy_and_notify_async(text);
    1
}
