# Nautilus Copy Path (Rust Native)

Module mở rộng tăng tốc tối đa chức năng **"Copy Path"** cho trình quản lý tệp Nautilus (GNOME Files) trên Ubuntu bằng Rust.

## Cấu trúc dự án
- `src/lib.rs`: Thư viện native C ABI (`libcopypath.so`) - xử lý sao chép clipboard Wayland/X11 và gửi Desktop notification trong background thread riêng biệt (độ trễ ~0.1ms, hoàn toàn không gây block hay đơ giao diện).
- `src/main.rs`: File thực thi dòng lệnh CLI (`copy-path <path>`).
- `Makefile` / `install.sh`: Tự động build và cài đặt thư viện vào hệ thống.

## Tối ưu hóa hiệu năng
1. **0.1ms C-ABI Call**: Kết nối trực tiếp giữa Nautilus và code Rust đã tối ưu (LTO + opt-level 3 + strip binary).
2. **Gdk Clipboard In-Memory**: Đồng bộ ngay trong RAM của tiến trình Nautilus.
3. **Detached Worker Thread**: Tác vụ gửi notification qua DBus và đồng bộ wl-copy được chạy ngầm độc lập, giao diện Nautilus phản hồi ngay tức thì (Zero-freeze).

## Lệnh build & cài đặt thủ công:
```bash
# Build release
cargo build --release

# Cài đặt vào ~/.local
make install
# hoặc chạy:
# cp target/release/libcopypath.so ~/.local/lib/
# cp target/release/copy-path ~/.local/bin/
# nautilus -q
```
