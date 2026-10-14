import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import GObject from 'gi://GObject';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';

const GUI_CMD = GLib.get_home_dir() + '/.local/bin/clippaste-gui';
const DAEMON_CMD = GLib.get_home_dir() + '/.local/bin/clippaste-daemon';

function getRuntimeDir() {
    const xdg = GLib.getenv('XDG_RUNTIME_DIR');
    if (xdg && xdg.trim().length > 0) {
        return xdg.trim() + '/clippaste';
    }
    return '/dev/shm/clippaste';
}

const ClipPasteIndicator = GObject.registerClass(
class ClipPasteIndicator extends PanelMenu.Button {
    _init() {
        // true: dontCreateMenu - biến button thành action button trực tiếp
        super._init(0.0, 'ClipPaste', true);

        this._icon = new St.Icon({
            icon_name: 'edit-paste-symbolic',
            style_class: 'system-status-icon clippaste-panel-icon',
        });
        this.add_child(this._icon);

        // Bắt sự kiện click chuột, chạm cảm ứng hoặc phím Enter/Space
        this.connect('event', (_actor, event) => {
            const eventType = event.type();
            if (
                eventType === Clutter.EventType.BUTTON_PRESS ||
                eventType === Clutter.EventType.TOUCH_BEGIN
            ) {
                this._openGui();
                return Clutter.EVENT_STOP;
            }
            if (eventType === Clutter.EventType.KEY_PRESS) {
                const symbol = event.get_key_symbol();
                if (
                    symbol === Clutter.KEY_Return ||
                    symbol === Clutter.KEY_KP_Enter ||
                    symbol === Clutter.KEY_space
                ) {
                    this._openGui();
                    return Clutter.EVENT_STOP;
                }
            }
            return Clutter.EVENT_PROPAGATE;
        });
    }

    _openGui() {
        try {
            GLib.spawn_command_line_async(GUI_CMD);
        } catch (e) {
            console.error('ClipPaste: Failed to spawn GUI:', e);
        }
    }
});

export default class ClipPasteExtension extends Extension {
    enable() {
        this._indicator = new ClipPasteIndicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator, 1, 'left');

        this._runtimeDir = getRuntimeDir();
        GLib.mkdir_with_parents(this._runtimeDir, 0o700);
        this._activeMarker = this._runtimeDir + '/gnome_ext.active';
        try {
            GLib.file_set_contents(this._activeMarker, new TextEncoder().encode('1'));
        } catch (_e) {}

        this._suppressNextCaptureUntil = 0;
        this._debounceTimeoutId = 0;

        // 1. Lắng nghe sự kiện copy trực tiếp từ Compositor (Mutter) - 0% CPU khi nghỉ, không dùng wl-paste
        this._selection = global.display.get_selection();
        this._ownerChangedId = this._selection.connect(
            'owner-changed',
            (_sel, selectionType, _selectionSource) => {
                if (selectionType === Meta.SelectionType.SELECTION_CLIPBOARD) {
                    if (GLib.get_monotonic_time() < this._suppressNextCaptureUntil) {
                        return;
                    }
                    this._scheduleCapture();
                }
            }
        );

        // 2. Theo dõi yêu cầu nạp lại clipboard khi dán (set_clip.json) trực tiếp qua St.Clipboard
        // để không cần gọi wl-copy (tránh cửa sổ tạm của wl-copy gây mất focus)
        this._setClipFile = Gio.File.new_for_path(this._runtimeDir + '/set_clip.json');
        try {
            this._clipMonitor = this._setClipFile.monitor_file(Gio.FileMonitorFlags.NONE, null);
            this._clipMonitorId = this._clipMonitor.connect(
                'changed',
                (_mon, _file, _other, eventType) => {
                    if (
                        eventType === Gio.FileMonitorEvent.CREATED ||
                        eventType === Gio.FileMonitorEvent.CHANGES_DONE_HINT ||
                        eventType === Gio.FileMonitorEvent.MOVED_IN ||
                        eventType === Gio.FileMonitorEvent.CHANGED
                    ) {
                        this._applyRequestedClipboard();
                    }
                }
            );
        } catch (e) {
            console.error('ClipPaste: Failed to monitor set_clip.json:', e);
        }
    }

    _applyRequestedClipboard() {
        try {
            const reqPath = this._runtimeDir + '/set_clip.json';
            if (!GLib.file_test(reqPath, GLib.FileTest.EXISTS)) {
                return;
            }
            const [ok, rawBytes] = GLib.file_get_contents(reqPath);
            if (!ok || !rawBytes || rawBytes.length === 0) {
                return;
            }
            try {
                this._setClipFile.delete(null);
            } catch (_e) {}

            const req = JSON.parse(new TextDecoder().decode(rawBytes));
            const clipboard = St.Clipboard.get_default();
            // Tạm ngưng bắt lại sự kiện do chính thao tác dán này tạo ra trong 400ms
            this._suppressNextCaptureUntil = GLib.get_monotonic_time() + 400000;

            if (req.item_type === 'text' && typeof req.content === 'string') {
                clipboard.set_text(St.ClipboardType.CLIPBOARD, req.content);
            } else if (req.item_type === 'image' && req.file_path) {
                const [imgOk, imgData] = GLib.file_get_contents(req.file_path);
                if (imgOk && imgData && imgData.length > 0) {
                    const gbytes = GLib.Bytes.new(imgData);
                    clipboard.set_content(
                        St.ClipboardType.CLIPBOARD,
                        req.mime_type || 'image/png',
                        gbytes
                    );
                }
            }
        } catch (e) {
            console.error('ClipPaste: Failed to set compositor clipboard:', e);
        }
    }

    _scheduleCapture() {
        if (this._debounceTimeoutId !== 0) {
            GLib.Source.remove(this._debounceTimeoutId);
            this._debounceTimeoutId = 0;
        }
        this._debounceTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 80, () => {
            this._debounceTimeoutId = 0;
            this._captureFromCompositor();
            return GLib.SOURCE_REMOVE;
        });
    }

    _captureFromCompositor() {
        try {
            const clipboard = St.Clipboard.get_default();
            const mimeTypes = clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD) || [];
            if (mimeTypes.length === 0) {
                return;
            }

            if (
                mimeTypes.some(
                    m =>
                        m.toLowerCase() === 'x-kde-passwordmanagerhint' ||
                        m.toLowerCase().includes('password')
                )
            ) {
                return;
            }

            const imgMime = ['image/png', 'image/webp', 'image/jpeg'].find(m =>
                mimeTypes.includes(m)
            );
            if (imgMime) {
                clipboard.get_content(St.ClipboardType.CLIPBOARD, imgMime, (_clip, bytes) => {
                    if (!bytes) return;
                    const data = bytes.get_data();
                    if (!data || data.length <= 100) return;
                    this._sendPayloadToDaemon('INGEST_IMAGE', 'ingest-image', data);
                });
                return;
            }

            if (
                mimeTypes.some(
                    m => m.startsWith('text/') || m === 'UTF8_STRING' || m === 'STRING'
                )
            ) {
                clipboard.get_text(St.ClipboardType.CLIPBOARD, (_clip, text) => {
                    if (!text || text.trim().length === 0) return;
                    const bytes = new TextEncoder().encode(text);
                    this._sendPayloadToDaemon('INGEST_TEXT', 'ingest-text', bytes);
                });
            }
        } catch (e) {
            console.error('ClipPaste: Failed to capture clipboard from compositor:', e);
        }
    }

    _sendPayloadToDaemon(ipcAction, cliSubcmd, uint8Data) {
        try {
            GLib.mkdir_with_parents(this._runtimeDir, 0o700);
            const stagedPath = `${this._runtimeDir}/staged_${GLib.get_monotonic_time()}.bin`;
            GLib.file_set_contents(stagedPath, uint8Data);

            const sockPath = `${this._runtimeDir}/daemon.sock`;
            if (GLib.file_test(sockPath, GLib.FileTest.EXISTS)) {
                try {
                    const address = Gio.UnixSocketAddress.new(sockPath);
                    const client = new Gio.SocketClient();
                    const conn = client.connect(address, null);
                    const out = conn.get_output_stream();
                    const msg = new TextEncoder().encode(`${ipcAction}\t${stagedPath}\n`);
                    out.write_all(msg, null);
                    conn.close(null);
                    return;
                } catch (_sockErr) {}
            }

            GLib.spawn_command_line_async(`${DAEMON_CMD} ${cliSubcmd} "${stagedPath}"`);
        } catch (e) {
            console.error('ClipPaste: Failed to send payload to daemon:', e);
        }
    }

    disable() {
        if (this._debounceTimeoutId !== 0) {
            GLib.Source.remove(this._debounceTimeoutId);
            this._debounceTimeoutId = 0;
        }
        if (this._clipMonitor) {
            if (this._clipMonitorId) {
                this._clipMonitor.disconnect(this._clipMonitorId);
                this._clipMonitorId = 0;
            }
            this._clipMonitor.cancel();
            this._clipMonitor = null;
        }
        if (this._selection && this._ownerChangedId) {
            this._selection.disconnect(this._ownerChangedId);
            this._ownerChangedId = 0;
            this._selection = null;
        }
        if (this._activeMarker) {
            try {
                const file = Gio.File.new_for_path(this._activeMarker);
                file.delete(null);
            } catch (_e) {}
        }
        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
    }
}
