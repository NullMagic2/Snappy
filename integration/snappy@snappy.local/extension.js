/* Module: snappy.shell_bridge. Purpose: authenticated screenshot and Mutter stream bridge.
 * Created: 2026-10-19. Architecture: only GNOME integration lives here; Rust owns UI,
 * audio, encoding, export and file destinations. No global security policy is changed.
 */
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const APP_ID = 'io.github.snappy.Capture';
const PATH = '/io/github/snappy/Capture';
const IFACE = 'io.github.snappy.Capture.Shell';
const MUTTER = 'org.gnome.Mutter.ScreenCast';
const SESSION = `${MUTTER}.Session`;
const STREAM = `${MUTTER}.Stream`;
const SCREEN_ONLY = 'snappy-screen-only';
const XML = `<node><interface name="${IFACE}">
<method name="Ping"><arg type="u" direction="out"/></method>
<method name="Screenshot"><arg type="ay" direction="out"/></method>
<method name="ScreenshotWithPointer"><arg type="b" direction="in"/><arg type="ay" direction="out"/></method>
<method name="StartStream"><arg type="o" direction="out"/><arg type="u" direction="out"/></method>
<method name="StartStreamWithPointer"><arg type="b" direction="in"/><arg type="o" direction="out"/><arg type="u" direction="out"/></method>
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

/* Paints its actor only into stage views, that is onto the screen. Mutter paints
 * screencast frames and screenshots into their own framebuffers, and this effect skips
 * those paints, so the recording bar stays visible above everything while recording
 * yet never appears in the recording. Picking is untouched, so Stop stays clickable.
 */
const ScreenOnly = GObject.registerClass(class SnappyScreenOnly extends Clutter.Effect {
    vfunc_paint(node, paintContext, flags) {
        const framebuffer = paintContext.get_framebuffer();
        const onScreen = this.get_actor().peek_stage_views()
            .some(view => view.get_framebuffer() === framebuffer);
        if (onScreen)
            super.vfunc_paint(node, paintContext, flags);
    }
});

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
        // Its actor is painted on screen only, so recordings do not contain the bar.
        // Identity and title can arrive after window-created, so they are checked
        // again whenever they change and when the window is first shown.
        this._windows = new Map();
        this._refreshSource = 0;
        this._windowCreated = global.display.connect('window-created', (_display, window) =>
            this._watchWindow(window));
        this._restacked = global.display.connect('restacked', () => this._queueRefresh());
        for (const actor of global.get_window_actors())
            this._watchWindow(actor.get_meta_window());
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
        if (this._restacked) global.display.disconnect(this._restacked);
        this._restacked = 0;
        if (this._refreshSource) global.compositor.get_laters().remove(this._refreshSource);
        this._refreshSource = 0;
        for (const [window, record] of this._windows) {
            for (const id of record.signals) window.disconnect(id);
            this._releaseWindow(window, record, true);
        }
        this._windows.clear();
        for (const path of this._sessions.keys())
            this._stop(path);
        this._object?.unexport();
        this._object = null;
        this._lockdown = null;
    }

    // Track existing windows as well as new ones: Shell may enable the extension
    // while Snappy is already recording. Keep every signal scoped to this enable.
    _watchWindow(window) {
        if (!window || this._windows.has(window))
            return;
        const record = {actor: null, wasAbove: false, signals: []};
        this._windows.set(window, record);
        for (const name of ['shown', 'notify::title', 'notify::wm-class', 'notify::gtk-application-id'])
            record.signals.push(window.connect(name, () => this._queueRefresh()));
        record.signals.push(window.connect('unmanaged', () => {
            for (const id of record.signals) window.disconnect(id);
            this._releaseWindow(window, record, false);
            this._windows.delete(window);
        }));
        this._queueRefresh();
    }

    _queueRefresh() {
        if (!this._enabled || this._refreshSource)
            return;
        // Wait until Mutter has finished mapping/restacking. Raising a window in
        // window-created can run before it has a stack position.
        this._refreshSource = global.compositor.get_laters().add(Meta.LaterType.BEFORE_REDRAW, () => {
            this._refreshSource = 0;
            for (const [window, record] of this._windows) {
                const snappy = window.get_gtk_application_id() === APP_ID ||
                    window.get_wm_class() === APP_ID;
                if (!snappy || window.get_title() !== 'Snappy recording') {
                    this._releaseWindow(window, record, true);
                    continue;
                }
                const actor = window.get_compositor_private();
                if (!actor?.is_mapped())
                    continue;
                if (!record.actor) {
                    record.actor = actor;
                    record.wasAbove = window.is_above();
                    actor.add_effect_with_name(SCREEN_ONLY, new ScreenOnly());
                    global.compositor.disable_unredirect();
                }
                if (!window.is_above()) window.make_above();
                this._moveActor(actor, global.top_window_group);
            }
            return GLib.SOURCE_REMOVE;
        });
    }

    _moveActor(actor, parent) {
        if (actor.get_parent() === parent)
            return;
        // Unmap before reparenting so Clutter does not mutate its pointer-device
        // table while it is updating picks for a mapped actor being removed.
        const visible = actor.visible;
        actor.hide();
        actor.get_parent()?.remove_child(actor);
        parent.add_child(actor);
        if (visible) actor.show();
    }

    _releaseWindow(window, record, restore) {
        const actor = record.actor;
        if (!actor)
            return;
        record.actor = null;
        // On unmanage Mutter owns actor destruction, including its effects.
        if (restore) {
            actor.remove_effect_by_name(SCREEN_ONLY);
            if (actor.get_parent() === global.top_window_group)
                this._moveActor(actor, global.window_group);
            if (!record.wasAbove) window.unmake_above();
        }
        global.compositor.enable_unredirect();
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
            invocation.return_value(new GLib.Variant('(u)', [3]));
        } catch (error) { this._error(invocation, error); }
    }

    ScreenshotAsync(_parameters, invocation) {
        return this.ScreenshotWithPointerAsync([false], invocation);
    }

    async ScreenshotWithPointerAsync([showPointer], invocation) {
        try {
            await this._authorize(invocation);
            const screenshot = new Shell.Screenshot();
            const output = Gio.MemoryOutputStream.new_resizable();
            // The original async method is used because Shell may already promisify it.
            const take = screenshot._original_screenshot ?? screenshot.screenshot;
            take.call(screenshot, showPointer, output, (object, result) => {
                try {
                    object.screenshot_finish(result);
                    output.close(null);
                    invocation.return_value(new GLib.Variant('(ay)', [output.steal_as_bytes().toArray()]));
                } catch (error) { this._error(invocation, error); }
            });
        } catch (error) { this._error(invocation, error); }
    }

    StartStreamAsync(_parameters, invocation) {
        return this.StartStreamWithPointerAsync([false], invocation);
    }

    async StartStreamWithPointerAsync([showPointer], invocation) {
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
                    'cursor-mode': new GLib.Variant('u', showPointer ? 1 : 0),
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
