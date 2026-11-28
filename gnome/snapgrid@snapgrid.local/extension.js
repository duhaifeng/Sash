// SnapGrid backend for GNOME Shell. Wayland gives a normal program no way to move
// other applications' windows or to receive global keys, so the Shell does it:
// this extension exports local.snapgrid.Shell on the session bus and the `snapgrid`
// program drives it. It holds no snapping logic of its own.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const BUS_NAME = 'local.snapgrid.Shell';
const OBJECT_PATH = '/local/snapgrid/Shell';
// Owned by the running `snapgrid` program; the extension follows it to know when the
// icon, shortcuts and drag handling should be live.
const DAEMON_NAME = 'local.snapgrid.Daemon';
// Reported by Version(); `snapgrid` compares it with the version it expects and asks
// the user to log in again when the Shell still runs an older copy. Keep it equal
// to EXTENSION_VERSION in src/gnome.rs and raise both when this file changes.
const EXTENSION_VERSION = 2;

const INTERFACE = `
<node>
  <interface name="local.snapgrid.Shell">
    <method name="Version">
      <arg type="u" direction="out" name="version"/>
    </method>
    <method name="ActiveWindow">
      <arg type="t" direction="out" name="id"/>
    </method>
    <method name="WindowInfo">
      <arg type="t" direction="in" name="id"/>
      <arg type="b" direction="out" name="exists"/>
      <arg type="b" direction="out" name="reserved"/>
      <arg type="s" direction="out" name="instance"/>
      <arg type="s" direction="out" name="class"/>
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
      <arg type="i" direction="out" name="w"/>
      <arg type="i" direction="out" name="h"/>
    </method>
    <method name="Exists">
      <arg type="t" direction="in" name="id"/>
      <arg type="b" direction="out" name="exists"/>
    </method>
    <method name="SetFrame">
      <arg type="t" direction="in" name="id"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="w"/>
      <arg type="i" direction="in" name="h"/>
    </method>
    <method name="ToggleAbove">
      <arg type="t" direction="in" name="id"/>
    </method>
    <method name="Monitors">
      <arg type="a(iiiiiiii)" direction="out" name="monitors"/>
    </method>
    <method name="Pointer">
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
    </method>
    <method name="WindowUnderPointer">
      <arg type="t" direction="out" name="id"/>
    </method>
    <method name="ShowOverlay">
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="w"/>
      <arg type="i" direction="in" name="h"/>
      <arg type="s" direction="in" name="fill"/>
      <arg type="s" direction="in" name="outline"/>
      <arg type="i" direction="in" name="outline_width"/>
    </method>
    <method name="HideOverlay"/>
    <method name="SetShortcuts">
      <arg type="as" direction="in" name="accelerators"/>
      <arg type="as" direction="out" name="failed"/>
    </method>
    <!-- The status menu's content, as JSON; see SnapGridIndicator._rebuild. -->
    <method name="SetMenu">
      <arg type="s" direction="in" name="json"/>
    </method>
    <method name="Notify">
      <arg type="s" direction="in" name="title"/>
      <arg type="s" direction="in" name="body"/>
    </method>
    <!-- Emitted when the extension (re)starts; the client must grab its shortcuts again. -->
    <signal name="Ready"/>
    <!-- A status menu item was chosen; window is the window that was active when the menu opened. -->
    <signal name="MenuAction">
      <arg type="s" name="action"/>
      <arg type="s" name="arg"/>
      <arg type="t" name="window"/>
    </signal>
    <signal name="Shortcut">
      <arg type="s" name="accelerator"/>
    </signal>
    <!-- "press" and "release" bracket a window move; "motion" repeats while it lasts. -->
    <signal name="Input">
      <arg type="s" name="kind"/>
    </signal>
  </interface>
</node>`;

// Windows other than these (docks, desktop, menus, ...) are never moved.
const MOVABLE_TYPES = [
    Meta.WindowType.NORMAL,
    Meta.WindowType.DIALOG,
    Meta.WindowType.MODAL_DIALOG,
    Meta.WindowType.UTILITY,
];
// META_GRAB_OP_WINDOW_FLAG_UNCONSTRAINED
const GRAB_OP_UNCONSTRAINED = 1024;
const MOTION_INTERVAL_MS = 16;

function reply(signature, values) {
    return new GLib.Variant(signature, values);
}

// The status-menu button. SnapGrid pushes the menu's content (SetMenu) and gets the
// chosen item back as a MenuAction signal; the menu is rebuilt each time it opens
// so it follows the active window's display and application.
const SnapGridIndicator = GObject.registerClass(
class SnapGridIndicator extends PanelMenu.Button {
    _init(service, iconPath) {
        super._init(0.5, 'SnapGrid');
        this._service = service;
        this.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(iconPath),
            style_class: 'system-status-icon',
        }));
        // The Shell does not open an empty menu, so it always holds something; the
        // real content replaces it on each opening.
        this._rebuild();
        this.menu.connect('open-state-changed', (_menu, open) => {
            if (open)
                this._rebuild();
        });
    }

    _separate() {
        const items = this.menu._getMenuItems();
        if (items.length === 0 || items[items.length - 1] instanceof PopupMenu.PopupSeparatorMenuItem)
            return;
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
    }

    _add(label, {shortcut = null, check = false, sensitive = true, activate = null} = {}) {
        const item = new PopupMenu.PopupMenuItem(label);
        if (shortcut) {
            const hint = new St.Label({
                text: shortcut,
                x_expand: true,
                x_align: Clutter.ActorAlign.END,
                y_align: Clutter.ActorAlign.CENTER,
                opacity: 150,
            });
            item.add_child(hint);
        }
        if (check)
            item.setOrnament(PopupMenu.Ornament.CHECK);
        item.setSensitive(sensitive);
        if (activate)
            item.connect('activate', activate);
        this.menu.addMenuItem(item);
        return item;
    }

    _rebuild() {
        const service = this._service;
        const model = service.menuModel;
        this.menu.removeAll();
        if (!model) {
            this._add('SnapGrid is starting…', {sensitive: false});
            return;
        }

        const window = service.menuTarget();
        const target = window?.get_id() ?? 0;
        const act = (action, arg = '') => service.emitMenuAction(action, arg, target);

        // Commands that apply to the display the window is on.
        const monitor = Main.layoutManager.monitors[
            window ? window.get_monitor() : Main.layoutManager.primaryIndex];
        const axis = !monitor || monitor.width >= monitor.height ? 'horizontal' : 'vertical';
        for (const command of model[axis] ?? []) {
            this._add(command.name, {
                shortcut: command.shortcut,
                activate: () => act('command', command.name),
            });
            if (command.separator)
                this._separate();
        }

        // Ignore the application of that window.
        if (window && service.isMovable(window)) {
            this._separate();
            const instance = (window.get_wm_class_instance() ?? '').toLowerCase();
            const cls = (window.get_wm_class() ?? '').toLowerCase();
            const listed = apps => (apps ?? []).some(app => {
                const name = app.toLowerCase();
                return name === instance || name === cls;
            });
            const app = Shell.WindowTracker.get_default().get_window_app(window);
            const name = app?.get_name() ?? window.get_wm_class() ?? window.get_title();
            if (listed(model.ignored_by_preferences)) {
                this._add(`“${name}” Is Ignored by Preferences`, {sensitive: false});
            } else {
                this._add(`Ignore “${name}”`, {
                    check: listed(model.ignored),
                    activate: () => act('ignore'),
                });
            }
        }

        this._separate();
        for (const [key, label] of [
            ['snap_windows', 'Snap Windows by Dragging'],
            ['restore_to_original_size', 'Restore Original Size When Dragging Away'],
        ]) {
            const item = new PopupMenu.PopupSwitchMenuItem(label, !!model[key]);
            item.connect('toggled', () => act('toggle', key));
            this.menu.addMenuItem(item);
        }

        this._separate();
        this._add('Settings…', {activate: () => service.openPreferences()});
        this._add('Reload Settings', {activate: () => act('reload')});
        this._separate();
        this._add('Quit SnapGrid', {activate: () => act('quit')});
    }
});

class SnapGridService {
    constructor() {
        this.menuModel = null;
        this._extension = null;
        this._actions = new Map();
        this._grabbed = null;
        this._motionTimer = 0;
        this._overlay = null;
        this._signals = [];
        this._ownerId = 0;
        this._watchId = 0;
        this._dbus = null;
        this._indicator = null;
        this._daemonUp = false;
        this._lastFocused = null;
    }

    start(extension) {
        this._extension = extension;
        this._dbus = Gio.DBusExportedObject.wrapJSObject(INTERFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._ownerId = Gio.bus_own_name_on_connection(
            Gio.DBus.session, BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);

        this._indicator = new SnapGridIndicator(this, `${extension.path}/icons/snapgrid-symbolic.svg`);
        Main.panel.addToStatusArea('snapgrid', this._indicator);
        this._syncIndicator();
        this._watchId = Gio.bus_watch_name(
            Gio.BusType.SESSION, DAEMON_NAME, Gio.BusNameWatcherFlags.NONE,
            () => {
                this._daemonUp = true;
                this._syncIndicator();
            },
            () => this._onDaemonGone());

        const display = global.display;
        this._lastFocused = display.focus_window;
        this._signals.push(
            display.connect('accelerator-activated', (_display, action) => this._onAccelerator(action)),
            display.connect('grab-op-begin', (_display, window, op) => this._onGrabBegin(window, op)),
            display.connect('grab-op-end', () => this._onGrabEnd()),
            display.connect('notify::focus-window', () => {
                if (display.focus_window)
                    this._lastFocused = display.focus_window;
            }));

        this._emit('Ready', null);
    }

    stop() {
        for (const id of this._signals)
            global.display.disconnect(id);
        this._signals = [];
        if (this._watchId) {
            Gio.bus_unwatch_name(this._watchId);
            this._watchId = 0;
        }
        this._indicator?.destroy();
        this._indicator = null;
        this._extension = null;
        this._stopMotion();
        this._grabbed = null;
        this._ungrabAll();
        if (this._overlay) {
            this._overlay.destroy();
            this._overlay = null;
        }
        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = 0;
        }
        if (this._dbus) {
            this._dbus.unexport();
            this._dbus = null;
        }
    }

    _emit(name, signature, ...args) {
        this._dbus?.emit_signal(name, signature ? new GLib.Variant(signature, args) : null);
    }

    _windows() {
        return global.get_window_actors().map(actor => actor.meta_window).filter(window => window);
    }

    _find(id) {
        return this._windows().find(window => window.get_id() === id) ?? null;
    }

    // -- status menu -------------------------------------------------------

    SetMenu(json) {
        try {
            this.menuModel = JSON.parse(json);
        } catch (error) {
            console.error(`snapgrid: invalid menu: ${error}`);
            return;
        }
        this._syncIndicator();
    }

    Notify(title, body) {
        Main.notify(title, body);
    }

    emitMenuAction(action, arg, window) {
        this._emit('MenuAction', '(sst)', action, arg, window);
    }

    // The window the menu acts on: the focused one, or the last one that had focus
    // when the menu itself has taken the focus.
    menuTarget() {
        const focused = global.display.focus_window;
        if (focused)
            return focused;
        const last = this._lastFocused;
        return last && this._windows().includes(last) ? last : null;
    }

    isMovable(window) {
        return MOVABLE_TYPES.includes(window.get_window_type());
    }

    // The settings page (prefs.js) talks to SnapGrid directly over D-Bus.
    openPreferences() {
        this._extension.openPreferences();
    }

    _syncIndicator() {
        if (this._indicator)
            this._indicator.visible = this._daemonUp && !this.menuModel?.hide_icon;
    }

    // `snapgrid` quit or died: nothing may keep swallowing its shortcuts or showing a
    // menu that goes nowhere.
    _onDaemonGone() {
        this._daemonUp = false;
        this.menuModel = null;
        this._indicator?.menu.close();
        this._stopMotion();
        this._grabbed = null;
        this._ungrabAll();
        this._overlay?.hide();
        this._syncIndicator();
    }

    // -- queries -----------------------------------------------------------

    Version() {
        return reply('(u)', [EXTENSION_VERSION]);
    }

    ActiveWindow() {
        return reply('(t)', [global.display.focus_window?.get_id() ?? 0]);
    }

    WindowInfo(id) {
        const window = this._find(id);
        if (!window)
            return reply('(bbssiiii)', [false, true, '', '', 0, 0, 0, 0]);
        const rect = window.get_frame_rect();
        return reply('(bbssiiii)', [
            true,
            !MOVABLE_TYPES.includes(window.get_window_type()),
            window.get_wm_class_instance() ?? '',
            window.get_wm_class() ?? '',
            rect.x, rect.y, rect.width, rect.height,
        ]);
    }

    Exists(id) {
        return reply('(b)', [this._find(id) !== null]);
    }

    Monitors() {
        const workspace = global.workspace_manager.get_active_workspace();
        const monitors = Main.layoutManager.monitors.map((monitor, index) => {
            const work = workspace.get_work_area_for_monitor(index);
            return [
                monitor.x, monitor.y, monitor.width, monitor.height,
                work.x, work.y, work.width, work.height,
            ];
        });
        return reply('(a(iiiiiiii))', [monitors]);
    }

    Pointer() {
        const [x, y] = global.get_pointer();
        return reply('(ii)', [x, y]);
    }

    WindowUnderPointer() {
        if (this._grabbed)
            return reply('(t)', [this._grabbed.get_id()]);
        const [x, y] = global.get_pointer();
        const workspace = global.workspace_manager.get_active_workspace();
        const actors = global.get_window_actors();
        // Window actors are ordered bottom to top.
        for (let i = actors.length - 1; i >= 0; i--) {
            const window = actors[i].meta_window;
            if (!window || window.minimized || !window.located_on_workspace(workspace))
                continue;
            const rect = window.get_frame_rect();
            if (x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height)
                return reply('(t)', [window.get_id()]);
        }
        return reply('(t)', [0]);
    }

    // -- actions -----------------------------------------------------------

    SetFrame(id, x, y, w, h) {
        const window = this._find(id);
        if (!window || !window.allows_move())
            return;
        if (window.is_fullscreen())
            window.unmake_fullscreen();
        // is_maximized() is only true when maximized both ways. A window tiled to
        // a half by the Shell (or Tiling Assistant) is maximized in one direction
        // only, and that direction then stays pinned to the work area, so a later
        // half or quarter would change only the other dimension.
        if (window.maximized_horizontally || window.maximized_vertically) {
            if (window.get_maximized)
                window.unmaximize(window.get_maximized());
            else
                window.unmaximize();
        }
        // Tiling Assistant also pins edges of tiled windows to the monitor.
        if (window.override_constraints) {
            window.override_constraints(Meta.WindowConstraint.NONE,
                Meta.WindowConstraint.NONE, Meta.WindowConstraint.NONE,
                Meta.WindowConstraint.NONE);
        }
        if (window.allows_resize())
            window.move_resize_frame(true, x, y, w, h);
        else
            window.move_frame(true, x, y);
    }

    ToggleAbove(id) {
        const window = this._find(id);
        if (!window)
            return;
        if (window.is_above())
            window.unmake_above();
        else
            window.make_above();
    }

    ShowOverlay(x, y, w, h, fill, outline, outlineWidth) {
        if (!this._overlay) {
            this._overlay = new St.Widget({reactive: false, can_focus: false, track_hover: false});
            Main.uiGroup.add_child(this._overlay);
        }
        this._overlay.set_style(
            `background-color: ${fill}; border: ${outlineWidth}px solid ${outline};`);
        this._overlay.set_position(x, y);
        this._overlay.set_size(w, h);
        this._overlay.show();
    }

    HideOverlay() {
        this._overlay?.hide();
    }

    // -- shortcuts ---------------------------------------------------------

    SetShortcuts(accelerators) {
        this._ungrabAll();
        const failed = [];
        for (const accelerator of accelerators) {
            const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
            if (action === Meta.KeyBindingAction.NONE) {
                failed.push(accelerator);
                continue;
            }
            // Like the Shell's own GrabAccelerator: without this the window
            // manager filters the binding out in every mode and it never fires.
            const binding = Meta.external_binding_name_for_action(action);
            Main.wm.allowKeybinding(binding, Shell.ActionMode.NORMAL);
            this._actions.set(action, {accelerator, binding});
        }
        return reply('(as)', [failed]);
    }

    _ungrabAll() {
        for (const [action, {binding}] of this._actions) {
            global.display.ungrab_accelerator(action);
            Main.wm.allowKeybinding(binding, Shell.ActionMode.NONE);
        }
        this._actions.clear();
    }

    _onAccelerator(action) {
        const grab = this._actions.get(action);
        if (grab === undefined || Main.sessionMode.isLocked)
            return;
        this._emit('Shortcut', '(s)', grab.accelerator);
    }

    // -- window drags ------------------------------------------------------

    _onGrabBegin(window, op) {
        if (!window || Main.sessionMode.isLocked)
            return;
        if ((op & ~GRAB_OP_UNCONSTRAINED) !== Meta.GrabOp.MOVING)
            return;
        this._grabbed = window;
        this._emit('Input', '(s)', 'press');
        this._motionTimer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, MOTION_INTERVAL_MS, () => {
            this._emit('Input', '(s)', 'motion');
            return GLib.SOURCE_CONTINUE;
        });
    }

    _onGrabEnd() {
        if (!this._grabbed)
            return;
        this._stopMotion();
        this._grabbed = null;
        this._emit('Input', '(s)', 'release');
    }

    _stopMotion() {
        if (this._motionTimer) {
            GLib.source_remove(this._motionTimer);
            this._motionTimer = 0;
        }
    }
}

export default class SnapGridExtension extends Extension {
    enable() {
        this._service = new SnapGridService();
        this._service.start(this);
    }

    disable() {
        this._service?.stop();
        this._service = null;
    }
}
