#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=== 0. Kiểm tra các gói phụ thuộc hệ thống ==="
MISSING_PKGS=()
command -v cargo >/dev/null 2>&1 || MISSING_PKGS+=("cargo")
command -v wl-paste >/dev/null 2>&1 || MISSING_PKGS+=("wl-clipboard")
python3 -c "import gi; gi.require_version('Gtk', '4.0'); gi.require_version('GdkPixbuf', '2.0')" 2>/dev/null || MISSING_PKGS+=("python3-gi" "gir1.2-gtk-4.0" "gir1.2-gdkpixbuf-2.0")

if [ ${#MISSING_PKGS[@]} -gt 0 ]; then
    echo "Phát hiện thiếu gói phụ thuộc: ${MISSING_PKGS[*]}"
    if command -v sudo >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
        echo "Đang cài đặt các gói phụ thuộc qua apt..."
        sudo apt-get update && sudo apt-get install -y "${MISSING_PKGS[@]}"
    else
        echo "Vui lòng cài đặt các gói sau trước khi tiếp tục: ${MISSING_PKGS[*]}"
        exit 1
    fi
fi

echo "=== 1. Biên dịch ClipPaste Rust Daemon ==="
cd "$SCRIPT_DIR/daemon"
cargo build --release

echo "=== 2. Dừng triệt để tiến trình cũ và cài đặt file thực thi vào ~/.local/bin ==="
systemctl --user stop clippaste.service 2>/dev/null || true
pkill -f "clippaste-daemon" 2>/dev/null || true
pkill -f "clippaste-gui --watch-x11" 2>/dev/null || true
pkill -f "wl-paste" 2>/dev/null || true

mkdir -p "$HOME/.local/bin"
cp "$SCRIPT_DIR/daemon/target/release/daemon" "$HOME/.local/bin/clippaste-daemon"
cp "$SCRIPT_DIR/gui/clippaste-gui" "$HOME/.local/bin/clippaste-gui"
chmod +x "$HOME/.local/bin/clippaste-daemon" "$HOME/.local/bin/clippaste-gui"

echo "=== 3. Cấu hình quyền /dev/uinput cho tính năng dán tự động (Ctrl+V) ==="
if [ ! -w /dev/uinput ]; then
    if command -v sudo >/dev/null 2>&1; then
        echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/99-clippaste-uinput.rules >/dev/null
        sudo udevadm control --reload-rules || true
        sudo udevadm trigger --sysname-match=uinput || true
    else
        echo "Lưu ý: Chưa có quyền ghi /dev/uinput. Hãy tạo /etc/udev/rules.d/99-clippaste-uinput.rules với TAG+=\"uaccess\"."
    fi
fi

echo "=== 4. Cài đặt GNOME Shell Extension ==="
EXT_DIR="$HOME/.local/share/gnome-shell/extensions/clippaste@dp.mod"
mkdir -p "$EXT_DIR"
cp "$SCRIPT_DIR/extension/metadata.json" "$EXT_DIR/"
cp "$SCRIPT_DIR/extension/extension.js" "$EXT_DIR/"
cp "$SCRIPT_DIR/extension/stylesheet.css" "$EXT_DIR/"

echo "=== 5. Cấu hình Systemd User Service tự khởi động ==="
mkdir -p "$HOME/.config/systemd/user"
cat << 'EOF' > "$HOME/.config/systemd/user/clippaste.service"
[Unit]
Description=ClipPaste Event-Driven Clipboard Daemon
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.local/bin/clippaste-daemon watch
ExecStopPost=/usr/bin/pkill -f "clippaste-gui --watch-x11"
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable clippaste.service
systemctl --user restart clippaste.service

echo "=== 6. Đăng ký Extension và Phím tắt Super + V vào gsettings ==="
python3 - << 'PYEOF'
import subprocess
import ast
import os

def gsettings_get(schema, key):
    raw = subprocess.check_output(['gsettings', 'get', schema, key]).decode('utf-8').strip()
    if raw.startswith('@as '):
        raw = raw[4:].strip()
    return ast.literal_eval(raw)

def gsettings_set(schema, key, val_repr):
    subprocess.check_call(['gsettings', 'set', schema, key, val_repr])

# 1. Kích hoạt GNOME Shell Extension
try:
    ext_list = gsettings_get('org.gnome.shell', 'enabled-extensions')
    if 'clippaste@dp.mod' not in ext_list:
        ext_list.append('clippaste@dp.mod')
        gsettings_set('org.gnome.shell', 'enabled-extensions', repr(ext_list))
except Exception as e:
    print(f"Lưu ý khi bật extension: {e}")

# 2. Giải phóng phím <Super>v mặc định của GNOME (toggle-message-tray) và gán cho ClipPaste GUI
try:
    tray_keys = gsettings_get('org.gnome.shell.keybindings', 'toggle-message-tray')
    new_tray_keys = [k for k in tray_keys if k.lower() != '<super>v']
    if new_tray_keys != tray_keys:
        gsettings_set('org.gnome.shell.keybindings', 'toggle-message-tray', repr(new_tray_keys))
except Exception:
    pass

try:
    media_schema = 'org.gnome.settings-daemon.plugins.media-keys'
    custom_path = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/clippaste/'
    custom_schema = f'{media_schema}.custom-keybinding:{custom_path}'

    bindings = gsettings_get(media_schema, 'custom-keybindings')
    if custom_path not in bindings:
        bindings.append(custom_path)
        gsettings_set(media_schema, 'custom-keybindings', repr(bindings))

    gui_cmd = os.path.expanduser('~/.local/bin/clippaste-gui')
    gsettings_set(custom_schema, 'name', "'ClipPaste'")
    gsettings_set(custom_schema, 'command', f"'{gui_cmd}'")
    gsettings_set(custom_schema, 'binding', "'<Super>v'")
except Exception as e:
    print(f"Lưu ý khi đăng ký phím tắt Super+V: {e}")
PYEOF

echo "=== Hoàn tất cài đặt ClipPaste ==="
echo "Lưu ý: Trên Wayland, nếu bạn vừa cập nhật GNOME Shell Extension, hãy đăng xuất (Log Out) và đăng nhập lại để GNOME Shell nạp bản extension mới."
