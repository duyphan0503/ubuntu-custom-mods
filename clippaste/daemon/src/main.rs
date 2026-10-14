use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_TEMP_ITEMS: usize = 50;
const LOCK_STALE_SECS: u64 = 5;

static LAST_CAPTURED_HASH: AtomicU64 = AtomicU64::new(0);
static LAST_GENERATED_ID: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

extern "C" {
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipItem {
    pub id: u64,
    pub item_type: String, // "text" or "image"
    pub content: Option<String>,
    pub file_path: Option<String>,
    pub preview: String,
    pub timestamp: u64,
    pub time_str: String,
    pub hash: u64,
    #[serde(default)]
    pub is_pinned: bool,
}

#[derive(Serialize)]
struct SetClipRequest<'a> {
    item_type: &'a str,
    content: Option<&'a str>,
    file_path: Option<&'a str>,
    mime_type: Option<&'a str>,
}

fn get_runtime_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return PathBuf::from(trimmed).join("clippaste");
        }
    }
    PathBuf::from("/dev/shm/clippaste")
}

fn get_socket_path() -> PathBuf {
    get_runtime_dir().join("daemon.sock")
}

fn get_ext_active_file() -> PathBuf {
    get_runtime_dir().join("gnome_ext.active")
}

fn get_set_clip_file() -> PathBuf {
    get_runtime_dir().join("set_clip.json")
}

fn get_temp_file() -> PathBuf {
    get_runtime_dir().join("temp.json")
}

fn get_history_file() -> PathBuf {
    get_runtime_dir().join("history.json")
}

fn get_shm_images_dir() -> PathBuf {
    get_runtime_dir().join("images")
}

fn get_last_hash_file() -> PathBuf {
    get_runtime_dir().join("last_hash")
}

fn get_lock_file() -> PathBuf {
    get_runtime_dir().join("state.lock")
}

fn get_pinned_dir() -> PathBuf {
    if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg_data.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("clippaste");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".local/share/clippaste")
}

fn get_pinned_file() -> PathBuf {
    get_pinned_dir().join("pinned.json")
}

fn get_pinned_images_dir() -> PathBuf {
    get_pinned_dir().join("pinned_images")
}

fn generate_unique_id() -> u64 {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    loop {
        let prev = LAST_GENERATED_ID.load(Ordering::SeqCst);
        let next = if now_ms > prev { now_ms } else { prev + 1 };
        if LAST_GENERATED_ID
            .compare_exchange(prev, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return next;
        }
    }
}

fn get_now_time_str() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    unsafe {
        let mut tm: Tm = std::mem::zeroed();
        if !localtime_r(&secs, &mut tm).is_null() {
            return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        }
    }
    format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
}

fn set_secure_dir_permissions(path: &Path) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

fn ensure_dirs() {
    let shm_images = get_shm_images_dir();
    let pinned_images = get_pinned_images_dir();
    if shm_images.exists() && pinned_images.exists() {
        return;
    }

    let runtime_dir = get_runtime_dir();
    let pinned_dir = get_pinned_dir();

    let _ = fs::create_dir_all(&shm_images);
    let _ = fs::create_dir_all(&pinned_images);
    set_secure_dir_permissions(&runtime_dir);
    set_secure_dir_permissions(&pinned_dir);

    let legacy_shm = Path::new("/dev/shm/clippaste");
    if runtime_dir != legacy_shm && !legacy_shm.exists() {
        let _ = std::os::unix::fs::symlink(&runtime_dir, legacy_shm);
    }
}

fn atomic_write(target: &Path, content: &[u8]) -> std::io::Result<()> {
    ensure_dirs();
    let parent = target.parent().unwrap_or_else(|| Path::new("/tmp"));
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("data");
    let tmp_path = parent.join(format!(".{}.tmp.{}", file_name, std::process::id()));

    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        f.write_all(content)?;
        f.flush()?;
    }
    fs::rename(&tmp_path, target)
}

struct StateLock {
    path: PathBuf,
}

impl StateLock {
    fn acquire() -> Self {
        ensure_dirs();
        let lock_path = get_lock_file();
        let start = Instant::now();

        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Self { path: lock_path };
                }
                Err(_) => {
                    if let Ok(meta) = fs::metadata(&lock_path) {
                        if let Ok(modified) = meta.modified() {
                            if modified.elapsed().unwrap_or_default().as_secs() >= LOCK_STALE_SECS {
                                let _ = fs::remove_file(&lock_path);
                                continue;
                            }
                        }
                    }
                    if start.elapsed() > Duration::from_secs(2) {
                        let _ = fs::remove_file(&lock_path);
                        return Self { path: lock_path };
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_same_as_last_hash(candidate_hash: u64) -> bool {
    if LAST_CAPTURED_HASH.load(Ordering::SeqCst) == candidate_hash {
        return true;
    }
    if let Ok(s) = fs::read_to_string(get_last_hash_file()) {
        if let Ok(file_hash) = s.trim().parse::<u64>() {
            if file_hash != 0 {
                LAST_CAPTURED_HASH.store(file_hash, Ordering::SeqCst);
                return file_hash == candidate_hash;
            }
        }
    }
    false
}

fn set_shared_last_hash(hash: u64) {
    LAST_CAPTURED_HASH.store(hash, Ordering::SeqCst);
    let _ = atomic_write(&get_last_hash_file(), hash.to_string().as_bytes());
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<Output> {
    let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                return child.wait_with_output().ok();
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn resolve_image_path(item_id: u64, current_path: Option<&str>) -> Option<String> {
    if let Some(p) = current_path {
        if Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    let pinned_path = get_pinned_images_dir().join(format!("{}.png", item_id));
    if pinned_path.exists() {
        return Some(pinned_path.to_string_lossy().to_string());
    }
    let shm_path = get_shm_images_dir().join(format!("{}.png", item_id));
    if shm_path.exists() {
        return Some(shm_path.to_string_lossy().to_string());
    }
    None
}

fn load_pinned() -> Vec<ClipItem> {
    let file = get_pinned_file();
    if file.exists() {
        if let Ok(data) = fs::read_to_string(&file) {
            if let Ok(mut items) = serde_json::from_str::<Vec<ClipItem>>(&data) {
                for item in &mut items {
                    item.is_pinned = true;
                    if item.item_type == "image" {
                        item.file_path = resolve_image_path(item.id, item.file_path.as_deref());
                    }
                }
                return items;
            }
        }
    }
    Vec::new()
}

fn save_pinned(items: &[ClipItem]) {
    if let Ok(json) = serde_json::to_string_pretty(items) {
        let _ = atomic_write(&get_pinned_file(), json.as_bytes());
    }
}

fn load_temp() -> Vec<ClipItem> {
    let temp_file = get_temp_file();
    if let Ok(data) = fs::read_to_string(&temp_file) {
        if let Ok(mut items) = serde_json::from_str::<Vec<ClipItem>>(&data) {
            for item in &mut items {
                item.is_pinned = false;
                if item.item_type == "image" {
                    item.file_path = resolve_image_path(item.id, item.file_path.as_deref());
                }
            }
            return items;
        }
    }
    Vec::new()
}

fn save_temp(items: &[ClipItem]) {
    if let Ok(json) = serde_json::to_string_pretty(items) {
        let _ = atomic_write(&get_temp_file(), json.as_bytes());
    }
}

pub fn get_combined_history() -> Vec<ClipItem> {
    let pinned = load_pinned();
    let temp = load_temp();

    let mut result = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for mut item in pinned {
        item.is_pinned = true;
        seen_ids.insert(item.id);
        result.push(item);
    }

    for mut item in temp {
        if !seen_ids.contains(&item.id) {
            item.is_pinned = false;
            seen_ids.insert(item.id);
            result.push(item);
        }
    }

    result
}

fn sync_to_shm() {
    let combined = get_combined_history();
    if let Ok(json) = serde_json::to_string_pretty(&combined) {
        let _ = atomic_write(&get_history_file(), json.as_bytes());
    }
}

fn enforce_temp_limit(temp: &mut Vec<ClipItem>, pinned: &[ClipItem]) {
    while temp.len() > MAX_TEMP_ITEMS {
        if let Some(removed) = temp.pop() {
            let still_pinned = pinned.iter().any(|p| p.id == removed.id);
            if !removed.is_pinned && !still_pinned {
                if let Some(p) = removed.file_path {
                    let _ = fs::remove_file(p);
                }
                let shm_file = get_shm_images_dir().join(format!("{}.png", removed.id));
                let _ = fs::remove_file(shm_file);
            }
        }
    }
}

fn get_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        Some((width, height))
    } else {
        None
    }
}

fn detect_image_mime(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        "image/png"
    } else if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff" {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn ingest_image_bytes(bytes: &[u8]) {
    if bytes.len() <= 100 {
        return;
    }
    let h = hash_bytes(bytes);
    if is_same_as_last_hash(h) {
        return;
    }

    let _lock = StateLock::acquire();
    let mut temp = load_temp();
    let pinned = load_pinned();

    if temp
        .first()
        .is_some_and(|first| first.item_type == "image" && first.hash == h)
        || pinned
            .iter()
            .any(|p| p.item_type == "image" && p.hash == h)
    {
        set_shared_last_hash(h);
        return;
    }

    if let Some(pos) = temp
        .iter()
        .position(|i| i.item_type == "image" && i.hash == h)
    {
        let mut existing = temp.remove(pos);
        existing.timestamp = generate_unique_id();
        existing.time_str = get_now_time_str();
        temp.insert(0, existing);
        set_shared_last_hash(h);
        save_temp(&temp);
        sync_to_shm();
        return;
    }

    let id = generate_unique_id();
    let file_path_buf = get_shm_images_dir().join(format!("{}.png", id));
    let file_path = file_path_buf.to_string_lossy().to_string();

    if fs::write(&file_path_buf, bytes).is_ok() {
        set_shared_last_hash(h);
        let preview = if let Some((w, h_dim)) = get_png_dimensions(bytes) {
            format!("Ảnh ({}x{})", w, h_dim)
        } else {
            format!("Ảnh ({:.1} KB)", bytes.len() as f64 / 1024.0)
        };

        let item = ClipItem {
            id,
            item_type: "image".to_string(),
            content: None,
            file_path: Some(file_path),
            preview,
            timestamp: id,
            time_str: get_now_time_str(),
            hash: h,
            is_pinned: false,
        };

        temp.insert(0, item);
        enforce_temp_limit(&mut temp, &pinned);
        save_temp(&temp);
        sync_to_shm();
    }
}

fn ingest_text_string(text: String) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    let h = hash_bytes(text.as_bytes());
    if is_same_as_last_hash(h) {
        return;
    }

    let _lock = StateLock::acquire();
    let mut temp = load_temp();
    let pinned = load_pinned();

    if temp
        .first()
        .is_some_and(|first| first.item_type == "text" && first.hash == h)
        || pinned
            .iter()
            .any(|p| p.item_type == "text" && p.hash == h)
    {
        set_shared_last_hash(h);
        return;
    }

    if let Some(pos) = temp
        .iter()
        .position(|i| i.item_type == "text" && i.hash == h)
    {
        let mut existing = temp.remove(pos);
        existing.timestamp = generate_unique_id();
        existing.time_str = get_now_time_str();
        temp.insert(0, existing);
        set_shared_last_hash(h);
        save_temp(&temp);
        sync_to_shm();
        return;
    }

    set_shared_last_hash(h);
    let id = generate_unique_id();
    let preview = if trimmed.chars().count() > 80 {
        let s: String = trimmed.chars().take(80).collect();
        format!("{}...", s.replace('\n', " "))
    } else {
        trimmed.replace('\n', " ")
    };

    let item = ClipItem {
        id,
        item_type: "text".to_string(),
        content: Some(text),
        file_path: None,
        preview,
        timestamp: id,
        time_str: get_now_time_str(),
        hash: h,
        is_pinned: false,
    };

    temp.insert(0, item);
    enforce_temp_limit(&mut temp, &pinned);
    save_temp(&temp);
    sync_to_shm();
}

fn handle_ingest_file(kind: &str, file_path: &str) {
    let path = Path::new(file_path);
    if !path.exists() {
        return;
    }
    if kind == "image" {
        if let Ok(bytes) = fs::read(path) {
            let _ = fs::remove_file(path);
            ingest_image_bytes(&bytes);
        }
    } else if kind == "text" {
        if let Ok(bytes) = fs::read(path) {
            let _ = fs::remove_file(path);
            let text = String::from_utf8_lossy(&bytes).to_string();
            ingest_text_string(text);
        }
    }
}

fn handle_capture() {
    ensure_dirs();

    let mut cmd = Command::new("wl-paste");
    cmd.arg("--list-types");
    let types_output = match run_with_timeout(cmd, Duration::from_millis(500)) {
        Some(out) => String::from_utf8_lossy(&out.stdout).to_string(),
        None => return,
    };

    let types: Vec<&str> = types_output.lines().map(|s| s.trim()).collect();
    if types.is_empty() {
        return;
    }

    if types
        .iter()
        .any(|&t| t.eq_ignore_ascii_case("x-kde-passwordManagerHint") || t.contains("password"))
    {
        return;
    }

    let preferred_img_mime = ["image/png", "image/webp", "image/jpeg"]
        .into_iter()
        .find(|&mime| types.contains(&mime));

    if let Some(img_mime) = preferred_img_mime {
        let mut cmd_img = Command::new("wl-paste");
        cmd_img.args(["--type", img_mime]);
        if let Some(output) = run_with_timeout(cmd_img, Duration::from_millis(1500)) {
            if output.stdout.len() > 100 {
                ingest_image_bytes(&output.stdout);
                return;
            }
        }
    }

    if types
        .iter()
        .any(|&t| t.starts_with("text/") || t == "UTF8_STRING" || t == "STRING")
    {
        let mut cmd_txt = Command::new("wl-paste");
        cmd_txt.arg("-n");
        if let Some(output) = run_with_timeout(cmd_txt, Duration::from_millis(500)) {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            ingest_text_string(text);
        }
    }
}

fn create_virtual_keyboard() -> Result<VirtualDevice, Box<dyn std::error::Error>> {
    let mut keys = AttributeSet::<KeyCode>::new();
    keys.insert(KeyCode::KEY_LEFTCTRL);
    keys.insert(KeyCode::KEY_V);

    let device = VirtualDevice::builder()?
        .name("ClipPaste Virtual Keyboard")
        .with_keys(&keys)?
        .build()?;
    Ok(device)
}

fn emit_ctrl_v_on_device(device: &mut VirtualDevice) -> Result<(), Box<dyn std::error::Error>> {
    let press_events = [
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTCTRL.code(), 1),
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_V.code(), 1),
    ];
    device.emit(&press_events)?;

    thread::sleep(Duration::from_millis(20));

    let release_events = [
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_V.code(), 0),
        InputEvent::new(EventType::KEY.0, KeyCode::KEY_LEFTCTRL.code(), 0),
    ];
    device.emit(&release_events)?;
    Ok(())
}

pub fn simulate_paste_ephemeral() -> Result<(), Box<dyn std::error::Error>> {
    let mut device = create_virtual_keyboard()?;
    thread::sleep(Duration::from_millis(160));
    emit_ctrl_v_on_device(&mut device)?;
    thread::sleep(Duration::from_millis(80));
    Ok(())
}

fn copy_bytes_to_wayland(mime_type: Option<&str>, bytes: &[u8]) {
    let mut cmd = Command::new("wl-copy");
    if let Some(mime) = mime_type {
        cmd.args(["--type", mime]);
    } else {
        cmd.arg("-n");
    }
    if let Ok(mut child) = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
        let _ = child.wait();
    }
}

fn perform_paste(item_id: u64, persistent_dev: &mut Option<VirtualDevice>) {
    let target = {
        let _lock = StateLock::acquire();
        let history = get_combined_history();
        match history.iter().find(|i| i.id == item_id) {
            Some(item) => item.clone(),
            None => return,
        }
    };

    set_shared_last_hash(target.hash);

    // 1. Set clipboard content:
    // Prefer native GNOME compositor clipboard via `set_clip.json` (zero wl-copy window popup)
    // when the GNOME Shell Extension is active, otherwise fall back to `wl-copy`.
    let ext_active = get_ext_active_file().exists();
    if target.item_type == "text" {
        if let Some(ref content) = target.content {
            if ext_active {
                let req = SetClipRequest {
                    item_type: "text",
                    content: Some(content.as_str()),
                    file_path: None,
                    mime_type: None,
                };
                if let Ok(json) = serde_json::to_vec(&req) {
                    let _ = atomic_write(&get_set_clip_file(), &json);
                }
            } else {
                copy_bytes_to_wayland(None, content.as_bytes());
            }
        }
    } else if target.item_type == "image" {
        if let Some(path) = resolve_image_path(target.id, target.file_path.as_deref()) {
            if let Ok(bytes) = fs::read(&path) {
                let mime = detect_image_mime(&bytes);
                if ext_active {
                    let req = SetClipRequest {
                        item_type: "image",
                        content: None,
                        file_path: Some(&path),
                        mime_type: Some(mime),
                    };
                    if let Ok(json) = serde_json::to_vec(&req) {
                        let _ = atomic_write(&get_set_clip_file(), &json);
                    }
                } else {
                    copy_bytes_to_wayland(Some(mime), &bytes);
                }
            }
        }
    }

    // 2. Wait 110ms so focus returns cleanly to the previously active window
    thread::sleep(Duration::from_millis(110));

    // 3. Emit Ctrl+V using the persistent VirtualDevice (no udev plug/unplug overhead)
    if persistent_dev.is_none() {
        if let Ok(dev) = create_virtual_keyboard() {
            thread::sleep(Duration::from_millis(150));
            *persistent_dev = Some(dev);
        }
    }

    let mut emitted_ok = false;
    if let Some(ref mut dev) = persistent_dev {
        if emit_ctrl_v_on_device(dev).is_ok() {
            emitted_ok = true;
        } else {
            *persistent_dev = None;
        }
    }

    if !emitted_ok {
        if let Err(e) = simulate_paste_ephemeral() {
            eprintln!("uinput paste failed: {}, trying fallbacks", e);
            let wtype_ok = Command::new("wtype")
                .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
                .status()
                .is_ok_and(|s| s.success());

            if !wtype_ok {
                let ydotool_ok = Command::new("ydotool")
                    .args(["key", "29:1", "47:1", "47:0", "29:0"])
                    .status()
                    .is_ok_and(|s| s.success());

                if !ydotool_ok {
                    let _ = Command::new("xdotool")
                        .args(["key", "--clearmodifiers", "ctrl+v"])
                        .spawn();
                }
            }
        }
    }
}

fn handle_pin_toggle(item_id: u64) {
    let _lock = StateLock::acquire();
    ensure_dirs();
    let mut pinned = load_pinned();
    let mut temp = load_temp();

    if let Some(pos) = pinned.iter().position(|i| i.id == item_id) {
        let mut unpinned = pinned.remove(pos);
        unpinned.is_pinned = false;

        if unpinned.item_type == "image" {
            let shm_path_buf = get_shm_images_dir().join(format!("{}.png", unpinned.id));
            let pinned_path_buf = get_pinned_images_dir().join(format!("{}.png", unpinned.id));

            if let Some(src) = resolve_image_path(unpinned.id, unpinned.file_path.as_deref()) {
                let src_path = Path::new(&src);
                if src_path != shm_path_buf && !shm_path_buf.exists() {
                    let _ = fs::copy(src_path, &shm_path_buf);
                }
            }
            if shm_path_buf.exists() && pinned_path_buf.exists() {
                let _ = fs::remove_file(&pinned_path_buf);
            }
            unpinned.file_path = Some(shm_path_buf.to_string_lossy().to_string());
        }

        temp.insert(0, unpinned);
        enforce_temp_limit(&mut temp, &pinned);
        save_pinned(&pinned);
        save_temp(&temp);
        sync_to_shm();
        return;
    }

    if let Some(pos) = temp.iter().position(|i| i.id == item_id) {
        let mut item = temp.remove(pos);
        item.is_pinned = true;

        if item.item_type == "image" {
            let dest_path = get_pinned_images_dir().join(format!("{}.png", item.id));
            let shm_path = get_shm_images_dir().join(format!("{}.png", item.id));
            if let Some(src) = resolve_image_path(item.id, item.file_path.as_deref()) {
                let src_path = Path::new(&src);
                if src_path != dest_path {
                    let _ = fs::copy(src_path, &dest_path);
                }
            }
            if dest_path.exists() && shm_path.exists() {
                let _ = fs::remove_file(&shm_path);
            }
            item.file_path = Some(dest_path.to_string_lossy().to_string());
        }

        pinned.insert(0, item);
        save_pinned(&pinned);
        save_temp(&temp);
        sync_to_shm();
    }
}

fn handle_delete(item_id: u64) {
    let _lock = StateLock::acquire();
    let mut pinned = load_pinned();
    let mut temp = load_temp();

    if let Some(pos) = pinned.iter().position(|i| i.id == item_id) {
        let item = pinned.remove(pos);
        if let Some(p) = item.file_path {
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_file(get_pinned_images_dir().join(format!("{}.png", item_id)));
        save_pinned(&pinned);
    }

    if let Some(pos) = temp.iter().position(|i| i.id == item_id) {
        let item = temp.remove(pos);
        if let Some(p) = item.file_path {
            let _ = fs::remove_file(p);
        }
        let _ = fs::remove_file(get_shm_images_dir().join(format!("{}.png", item_id)));
        save_temp(&temp);
    }

    sync_to_shm();
}

fn handle_clear() {
    let _lock = StateLock::acquire();
    let _ = fs::remove_dir_all(get_shm_images_dir());
    ensure_dirs();
    save_temp(&[]);
    sync_to_shm();
}

fn send_ipc_command(cmd_line: &str) -> bool {
    let sock_path = get_socket_path();
    if let Ok(mut stream) = UnixStream::connect(&sock_path) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        if stream.write_all(cmd_line.as_bytes()).is_ok() {
            return true;
        }
    }
    false
}

fn spawn_x11_event_bridge() -> Option<Child> {
    let home = env::var("HOME").unwrap_or_default();
    let gui_path = PathBuf::from(&home).join(".local/bin/clippaste-gui");
    let bin = if gui_path.exists() {
        gui_path.to_string_lossy().to_string()
    } else {
        "clippaste-gui".to_string()
    };

    Command::new(bin)
        .arg("--watch-x11")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn handle_watch() {
    println!("ClipPaste Daemon started (Event-Driven Socket Server, 0% idle CPU)...");
    ensure_dirs();
    {
        let _lock = StateLock::acquire();
        sync_to_shm();
    }

    let sock_path = get_socket_path();
    let _ = fs::remove_file(&sock_path);

    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind IPC socket {:?}: {}", sock_path, e);
            return;
        }
    };
    let _ = fs::set_permissions(&sock_path, fs::Permissions::from_mode(0o600));

    let mut persistent_vdev = create_virtual_keyboard().ok();
    let mut x11_bridge = spawn_x11_event_bridge();

    for stream_res in listener.incoming() {
        if let Some(ref mut child) = x11_bridge {
            if child.try_wait().ok().flatten().is_some() {
                x11_bridge = None;
            }
        }

        if let Ok(stream) = stream_res {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                let parts: Vec<&str> = line.trim().splitn(2, '\t').collect();
                let action = parts.first().copied().unwrap_or("");
                let arg = parts.get(1).copied().unwrap_or("");

                match action {
                    "INGEST_TEXT" => handle_ingest_file("text", arg),
                    "INGEST_IMAGE" => handle_ingest_file("image", arg),
                    "PASTE" => {
                        if let Ok(id) = arg.parse::<u64>() {
                            perform_paste(id, &mut persistent_vdev);
                        }
                    }
                    "PIN" => {
                        if let Ok(id) = arg.parse::<u64>() {
                            handle_pin_toggle(id);
                        }
                    }
                    "DELETE" => {
                        if let Ok(id) = arg.parse::<u64>() {
                            handle_delete(id);
                        }
                    }
                    "CLEAR" => handle_clear(),
                    _ => {}
                }
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("watch");

    match cmd {
        "capture" => handle_capture(),
        "ingest-text" => {
            if let Some(path) = args.get(2) {
                if !send_ipc_command(&format!("INGEST_TEXT\t{}\n", path)) {
                    handle_ingest_file("text", path);
                }
            }
        }
        "ingest-image" => {
            if let Some(path) = args.get(2) {
                if !send_ipc_command(&format!("INGEST_IMAGE\t{}\n", path)) {
                    handle_ingest_file("image", path);
                }
            }
        }
        "paste" => {
            if let Some(id_str) = args.get(2) {
                if !send_ipc_command(&format!("PASTE\t{}\n", id_str)) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        let mut dev = None;
                        perform_paste(id, &mut dev);
                    }
                }
            }
        }
        "pin" => {
            if let Some(id_str) = args.get(2) {
                if !send_ipc_command(&format!("PIN\t{}\n", id_str)) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        handle_pin_toggle(id);
                    }
                }
            }
        }
        "delete" => {
            if let Some(id_str) = args.get(2) {
                if !send_ipc_command(&format!("DELETE\t{}\n", id_str)) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        handle_delete(id);
                    }
                }
            }
        }
        "clear" => {
            if !send_ipc_command("CLEAR\n") {
                handle_clear();
            }
        }
        "list" => {
            let items = get_combined_history();
            println!(
                "{}",
                serde_json::to_string_pretty(&items).unwrap_or_default()
            );
        }
        "watch" => handle_watch(),
        _ => {
            eprintln!(
                "Usage: clippaste-daemon [watch | capture | ingest-text <file> | ingest-image <file> | paste <id> | pin <id> | delete <id> | clear | list]"
            );
        }
    }
}
