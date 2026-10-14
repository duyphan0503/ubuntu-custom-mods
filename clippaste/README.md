# ClipPaste - Quản lý Clipboard Tạm thời (Text & Ảnh) cho Ubuntu 26.04 LTS

Tính năng lưu trữ tạm thời các đoạn văn bản (Text) và hình ảnh (Image) vừa copy, hiển thị icon trên thanh topbar (góc trái, ngay cạnh indicator workspace), hỗ trợ click để dán trực tiếp vào ô input có con trỏ chuột.

---

## Cấu trúc dự án

```text
clippaste/
├── daemon/                 # Rust Daemon (clippaste-daemon)
│   ├── Cargo.toml          # Tối ưu release (LTO, opt-level 3)
│   └── src/main.rs         # Bắt clipboard text & image, ghim lưu trữ, xóa, giả lập Ctrl+V
├── extension/              # GNOME Shell Extension (clippaste@dp.mod)
│   ├── metadata.json       # Tương thích GNOME Shell 45 -> 50
│   ├── extension.js        # Đặt icon góc trái topbar (vị trí 1 'left' ngay cạnh workspace pill)
│   └── stylesheet.css      # Render thumbnail ảnh, text preview, nút ghim Pin
├── gui/                    # Giao diện GTK4 độc lập (clippaste-gui)
│   └── clippaste-gui       # Cửa sổ popup tìm kiếm, click dòng dán ngay, vuốt trái để xóa, toggle mở/đóng
└── install.sh              # Script build và cài đặt trọn gói
```

---

## Các tính năng nổi bật:

1. **Bỏ nút "Dán" - Click vào dòng là dán luôn**:
   - Bạn chỉ cần click chuột vào bất kỳ dòng nào trong danh sách, cửa sổ sẽ lập tức đóng lại và dán thẳng nội dung vào ô input nơi con trỏ chuột đang đứng.

2. **Nút "Pin" (Ghim) lưu trữ vĩnh viễn**:
   - Sử dụng icon chuẩn GNOME `view-pin-symbolic`.
   - **Mục chưa ghim**: Lưu trong `/dev/shm` (RAM ảo) - tự động xóa sạch khi tắt máy hoặc reboot.
   - **Mục đã ghim**: Được lưu sang `~/.local/share/clippaste/pinned.json` và thư mục ảnh tĩnh `pinned_images/`, **không bao giờ bị mất khi khởi động lại máy hay reset hệ thống**. Các mục đã ghim luôn được ưu tiên hiển thị ở trên đầu danh sách kèm ký hiệu `📌`.

3. **Vuốt từ phải qua trái để xóa từng dòng (Swipe to Delete)**:
   - Trên touchpad hoặc kéo chuột từ phải sang trái (kéo > 80px), dòng nội dung sẽ trượt sang trái làm lộ biểu tượng thùng rác màu đỏ và xóa ngay mục đó ra khỏi lịch sử.

4. **Phím tắt nhanh `Super + V` (Toggle Mở / Đóng)**:
   - Bấm `Super + V` lần 1 để mở cửa sổ danh sách.
   - Bấm `Super + V` lần 2 (hoặc bấm lại icon Topbar) để đóng ngay lập tức.
   - Hỗ trợ phím `Esc` để đóng nhanh.
