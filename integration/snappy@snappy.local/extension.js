/* Module: snappy.shell_bridge. Purpose: authenticated screenshot and Mutter stream bridge.
 * Created: 2026-10-19. Architecture: only GNOME integration lives here; Rust owns UI,
 * audio, encoding, export and file destinations. No global security policy is changed.
 */
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const APP_ID = 'io.github.snappy.Capture';
const PATH = '/io/github/snappy/Capture';
const IFACE = 'io.github.snappy.Capture.Shell';
const MUTTER = 'org.gnome.Mutter.ScreenCast';
const SESSION = `${MUTTER}.Session`;
const STREAM = `${MUTTER}.Stream`;
const XML = `<node><interface name="${IFACE}">
<method name="Ping"><arg type="u" direction="out"/></method>
<method name="Screenshot"><arg type="ay" direction="out"/></method>
<method name="StartStream"><arg type="o" direction="out"/><arg type="u" direction="out"/></method>
<method name="StopStream"><arg type="o" direction="in"/></method>
</interface></node>`;

function call(destination, path, iface, method, parameters, replyType = null) {
    return new Promise((resolve, reject) => Gio.DBus.session.call(
        destination, path, iface, method, parameters,
        replyType ? new GLib.VariantType(replyType) : null,
        Gio.DBusCallFlags.NONE, 5000, null,
        (connection, result) => {
            try { resolve(connection.call_finish(result).deepUnpack()); }
            catch (error) { reject(error); }
        }));
}

export default class SnappyIntegration extends Extension {
    enable() {
        this._sessions = new Map();
        this._enabled = true;
        this._animations = new InjectionManager();
        this._animations.overrideMethod(Main.wm, '_shouldAnimateActor', original => function (actor, ...args) {
            const window = actor.get_meta_window();
            const snappy = window && (window.get_gtk_application_id() === APP_ID ||
                window.get_wm_class() === APP_ID);
            if (snappy && window.get_title() === 'Snappy capture overlay')
                return false;
            return original.call(this, actor, ...args);
        });
        // Wayland clients cannot ask to stay above other windows, and even an "above"
        // window sits below a focused fullscreen one (a game, a video). The recording
        // bar is therefore lifted into the group above every window layer, and
        // fullscreen unredirection stays off while it exists so it is always composited.
        // Identity and title can arrive after window-created, so they are checked
        // again whenever they change and when the window is first shown.
        this._lifted = new Set();
        this._windowCreated = global.display.connect('window-created', (_display, window) => {
            let lifted = false;
            const lift = () => {
                if (!this._enabled || lifted)
                    return;
                const snappy = window.get_gtk_application_id() === APP_ID ||
                    window.get_wm_class() === APP_ID;
                if (!snappy || window.get_title() !== 'Snappy recording')
                    return;
                const actor = window.get_compositor_private();
                if (!actor)
                    return;
                lifted = true;
                window.make_above();
                actor.get_parent()?.remove_child(actor);
                global.top_window_group.add_child(actor);
                global.compositor.disable_unredirect();
                this._lifted.add(actor);
            };
            const signals = ['shown', 'notify::title', 'notify::wm-class', 'notify::gtk-application-id']
                .map(name => window.connect(name, lift));
            window.connect('unmanaged', () => {
                for (const id of signals)
                    window.disconnect(id);
                if (!lifted)
                    return;
                const actor = window.get_compositor_private();
                if (actor)
                    this._lifted.delete(actor);
                if (this._enabled)
                    global.compositor.enable_unredirect();
            });
            lift();
        });
        this._lockdown = new Gio.Settings({schema_id: 'org.gnome.desktop.lockdown'});
        this._object = Gio.DBusExportedObject.wrapJSObject(XML, this);
        this._object.export(Gio.DBus.session, PATH);
        this._sessionModeSignal = Main.sessionMode.connect('updated', () => {
            if (Main.sessionMode.isLocked)
                for (const path of this._sessions.keys()) this._stop(path);
        });
    }

    disable() {
        this._enabled = false;
        this._animations?.clear();
        this._animations = null;
        if (this._sessionModeSignal) Main.sessionMode.disconnect(this._sessionModeSignal);
        this._sessionModeSignal = 0;
        if (this._windowCreated) global.display.disconnect(this._windowCreated);
        this._windowCreated = 0;
        for (const actor of this._lifted ?? []) {
            actor.get_parent()?.remove_child(actor);
            global.window_group.add_child(actor);
            global.compositor.enable_unredirect();
        }
        this._lifted = null;
        for (const path of this._sessions.keys())
            this._stop(path);
        this._object?.unexport();
        this._object = null;
        this._lockdown = null;
    }

    async _authorize(invocation) {
        if (!this._enabled)
            throw new Error('Snappy integration is disabled');
        const sender = invocation.get_sender();
        const busCall = (method, name, type) => call(
            'org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', method, new GLib.Variant('(s)', [name]), type);
        const [owner] = await busCall('GetNameOwner', APP_ID, '(s)');
        const [ownerPid] = await busCall('GetConnectionUnixProcessID', owner, '(u)');
        const [senderPid] = await busCall('GetConnectionUnixProcessID', sender, '(u)');
        const executable = GLib.file_read_link(`/proc/${senderPid}/exe`).replace(/ \(deleted\)$/, '');
        if (ownerPid !== senderPid || GLib.path_get_basename(executable) !== 'snappy')
            throw new Error('Only the active Snappy application can use this integration');
        if (Main.sessionMode.isLocked || Main.screenShield?.locked ||
            this._lockdown.get_boolean('disable-save-to-disk'))
            throw new Error('Screen capture is unavailable while locked or disabled by policy');
        if (!this._enabled)
            throw new Error('Snappy integration was disabled');
        return sender;
    }

    _error(invocation, error) {
        invocation.return_dbus_error(`${IFACE}.Error`, error.message ?? String(error));
    }

    async PingAsync(_parameters, invocation) {
        try {
            await this._authorize(invocation);
            invocation.return_value(new GLib.Variant('(u)', [1]));
        } catch (error) { this._error(invocation, error); }
    }

    async ScreenshotAsync(_parameters, invocation) {
        try {
            await this._authorize(invocation);
            const screenshot = new Shell.Screenshot();
            const output = Gio.MemoryOutputStream.new_resizable();
            // The original async method is used because Shell may already promisify it.
            const take = screenshot._original_screenshot ?? screenshot.screenshot;
            take.call(screenshot, false, output, (object, result) => {
                try {
                    object.screenshot_finish(result);
                    output.close(null);
                    invocation.return_value(new GLib.Variant('(ay)', [output.steal_as_bytes().toArray()]));
                } catch (error) { this._error(invocation, error); }
            });
        } catch (error) { this._error(invocation, error); }
    }

    async StartStreamAsync(_parameters, invocation) {
        let path;
        try {
            const sender = await this._authorize(invocation);
            if ([...this._sessions.values()].some(session => session.sender === sender))
                throw new Error('Snappy already has an active recording stream');
            [path] = await call(MUTTER, '/org/gnome/Mutter/ScreenCast', MUTTER,
                'CreateSession', new GLib.Variant('(a{sv})', [{}]), '(o)');
            const record = {sender, watch: 0, subscription: 0, timeout: 0};
            this._sessions.set(path, record);
            record.watch = Gio.bus_watch_name_on_connection(Gio.DBus.session, sender,
                Gio.BusNameWatcherFlags.NONE, null, () => this._stop(path));
            const [stream] = await call(MUTTER, path, SESSION, 'RecordArea',
                new GLib.Variant('(iiiia{sv})', [0, 0, global.stage.width, global.stage.height, {
                    'cursor-mode': new GLib.Variant('u', 0),
                    'is-recording': new GLib.Variant('b', true),
                }]), '(o)');
            const node = await new Promise((resolve, reject) => {
                record.reject = reject;
                record.subscription = Gio.DBus.session.signal_subscribe(MUTTER, STREAM,
                    'PipeWireStreamAdded', stream, null, Gio.DBusSignalFlags.NONE,
                    (_connection, _sender, _path, _iface, _signal, parameters) => {
                        resolve(parameters.deepUnpack()[0]);
                    });
                record.timeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10000, () => {
                    record.timeout = 0;
                    reject(new Error('Timed out starting the native recording stream'));
                    return GLib.SOURCE_REMOVE;
                });
                call(MUTTER, path, SESSION, 'Start', null, '()').catch(reject);
            });
            this._clearPending(record);
            if (!this._enabled || !this._sessions.has(path))
                throw new Error('Native recording stream was closed');
            invocation.return_value(new GLib.Variant('(ou)', [path, node]));
        } catch (error) {
            if (path) this._stop(path);
            this._error(invocation, error);
        }
    }

    async StopStreamAsync([path], invocation) {
        try {
            const sender = await this._authorize(invocation);
            const record = this._sessions.get(path);
            if (record && record.sender !== sender)
                throw new Error('The stream belongs to another application');
            this._stop(path);
            invocation.return_value(new GLib.Variant('()', []));
        } catch (error) { this._error(invocation, error); }
    }

    _clearPending(record) {
        if (record.subscription) Gio.DBus.session.signal_unsubscribe(record.subscription);
        if (record.timeout) GLib.source_remove(record.timeout);
        record.subscription = record.timeout = 0;
        record.reject = null;
    }

    _stop(path) {
        const record = this._sessions.get(path);
        if (!record) return;
        this._sessions.delete(path);
        const reject = record.reject;
        this._clearPending(record);
        reject?.(new Error('Native recording stream was closed'));
        if (record.watch) Gio.bus_unwatch_name(record.watch);
        call(MUTTER, path, SESSION, 'Stop', null, '()').catch(() => {});
    }
}
