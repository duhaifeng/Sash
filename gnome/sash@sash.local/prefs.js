// Sash settings. This runs in the Extensions app, not in the Shell: every
// change goes to the running `sash` over D-Bus (local.sash.Daemon), which
// applies it at once and writes config.toml / the commands file.

import Adw from 'gi://Adw';
import Gdk from 'gi://Gdk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

const NAME = 'local.sash.Daemon';
const PATH = '/local/sash/Daemon';
const IFACE = 'local.sash.Daemon';
const REPLY = new GLib.VariantType('(s)');
const TIMEOUT_MS = 15000;
const DEBOUNCE_MS = 300;

// Landscape displays use a 24x12 grid, portrait displays 12x24.
const AXES = [
    {key: 'horizontal', title: 'Landscape', icon: 'video-display-symbolic', cols: 24, rows: 12,
        about: 'Commands for displays that are wider than tall, on a 24 × 12 grid.'},
    {key: 'vertical', title: 'Portrait', icon: 'phone-symbolic', cols: 12, rows: 24,
        about: 'Commands for displays that are taller than wide, on a 12 × 24 grid.'},
];

const SPECIALS = {
    next_display: 'Moves the window to the next display',
    previous_display: 'Moves the window to the previous display',
    center: 'Centres the window without resizing it',
    restore: 'Restores the size and position before snapping',
    stay_on_top: 'Keeps the window above others',
};

Gio._promisify(Gio.DBusConnection.prototype, 'call', 'call_finish');

function variant(signature, values) {
    return new GLib.Variant(signature, values);
}

async function ask(method, args = null) {
    const reply = await Gio.DBus.session.call(NAME, PATH, IFACE, method, args, REPLY,
        Gio.DBusCallFlags.NONE, TIMEOUT_MS, null);
    return JSON.parse(reply.deepUnpack()[0]);
}

function askSync(method, args = null) {
    const reply = Gio.DBus.session.call_sync(NAME, PATH, IFACE, method, args, REPLY,
        Gio.DBusCallFlags.NONE, TIMEOUT_MS, null);
    return JSON.parse(reply.deepUnpack()[0]);
}

function messageOf(error) {
    try {
        Gio.DBusError.strip_remote_error(error);
    } catch (_e) {
        // Not a remote error.
    }
    return error?.message ?? String(error);
}

function clear(container) {
    let child;
    while ((child = container.get_first_child()))
        container.remove(child);
}

const toArray = rect => [[rect[0], rect[1]], [rect[2], rect[3]]];
const toRect = area => [area[0][0], area[0][1], area[1][0], area[1][1]];

// "Ctrl+Alt+Left" as GTK writes accelerators.
function acceleratorOf(shortcut) {
    const parts = shortcut.split('+');
    const key = parts.pop();
    const names = {ctrl: '<Control>', alt: '<Alt>', shift: '<Shift>', super: '<Super>'};
    return parts.map(part => names[part.toLowerCase()] ?? '').join('') +
        (key.length === 1 ? key.toLowerCase() : key);
}

function accentColor() {
    try {
        return Adw.StyleManager.get_default().get_accent_color_rgba();
    } catch (_e) {
        return {red: 0.21, green: 0.52, blue: 0.89};
    }
}

// A grid on which a rectangle of cells is picked by dragging.
class GridPicker {
    constructor({cols, rows, rect, context = null, color, cell = 12, onChange}) {
        this._cols = cols;
        this._rows = rows;
        this._rect = rect;
        this._context = context;
        this._color = color;
        this._cell = cell;
        this._onChange = onChange;
        this._drag = null;

        this.widget = new Gtk.DrawingArea({
            content_width: cols * cell,
            content_height: rows * cell,
            halign: Gtk.Align.START,
            valign: Gtk.Align.START,
        });
        this.widget.set_cursor_from_name('crosshair');
        this.widget.set_draw_func((area, cr, width, height) => this._draw(area, cr, width, height));

        const gesture = new Gtk.GestureDrag();
        gesture.connect('drag-begin', (_g, x, y) => {
            const from = this._cellAt(x, y);
            this._drag = {origin: [x, y], from, to: from};
            this.widget.queue_draw();
        });
        gesture.connect('drag-update', (_g, dx, dy) => {
            if (!this._drag)
                return;
            this._drag.to = this._cellAt(this._drag.origin[0] + dx, this._drag.origin[1] + dy);
            this.widget.queue_draw();
        });
        gesture.connect('drag-end', () => {
            if (!this._drag)
                return;
            this._rect = this._span(this._drag.from, this._drag.to);
            this._drag = null;
            this.widget.queue_draw();
            this._onChange(this._rect);
        });
        this.widget.add_controller(gesture);
    }

    _cellAt(x, y) {
        const clamp = (value, max) => Math.max(0, Math.min(max - 1, value));
        return [clamp(Math.floor(x / this._cell), this._cols), clamp(Math.floor(y / this._cell), this._rows)];
    }

    _span(a, b) {
        return [Math.min(a[0], b[0]), Math.min(a[1], b[1]),
            Math.abs(a[0] - b[0]) + 1, Math.abs(a[1] - b[1]) + 1];
    }

    _draw(area, cr, width, height) {
        const cell = this._cell;
        const fg = area.get_color();
        cr.setSourceRGBA(fg.red, fg.green, fg.blue, 0.08);
        cr.rectangle(0, 0, width, height);
        cr.fill();

        cr.setLineWidth(1);
        cr.setSourceRGBA(fg.red, fg.green, fg.blue, 0.18);
        for (let x = 1; x < this._cols; x++) {
            cr.moveTo(x * cell + 0.5, 0);
            cr.lineTo(x * cell + 0.5, height);
        }
        for (let y = 1; y < this._rows; y++) {
            cr.moveTo(0, y * cell + 0.5);
            cr.lineTo(width, y * cell + 0.5);
        }
        cr.stroke();

        const accent = accentColor();
        if (this._context) {
            const [x, y, w, h] = this._context;
            cr.setLineWidth(2);
            cr.setSourceRGBA(accent.red, accent.green, accent.blue, 0.6);
            cr.rectangle(x * cell + 1, y * cell + 1, w * cell - 2, h * cell - 2);
            cr.stroke();
        }

        const shown = this._drag ? this._span(this._drag.from, this._drag.to) : this._rect;
        if (shown) {
            const [x, y, w, h] = shown;
            // The activation areas use the text colour so that they never match the accent.
            const color = this._color === 'foreground' ? fg : accent;
            cr.setSourceRGBA(color.red, color.green, color.blue, 0.45);
            cr.rectangle(x * cell, y * cell, w * cell, h * cell);
            cr.fill();
            cr.setLineWidth(2);
            cr.setSourceRGBA(color.red, color.green, color.blue, 1);
            cr.rectangle(x * cell + 1, y * cell + 1, w * cell - 2, h * cell - 2);
            cr.stroke();
        }
        cr.$dispose();
    }
}

// Keys as Sash names them, and the modifiers, from a key press.
function shortcutFromKey(widget, controller, keycode, state) {
    const mods = [];
    if (state & Gdk.ModifierType.CONTROL_MASK)
        mods.push('Ctrl');
    if (state & Gdk.ModifierType.ALT_MASK)
        mods.push('Alt');
    if (state & Gdk.ModifierType.SHIFT_MASK)
        mods.push('Shift');
    if (state & Gdk.ModifierType.SUPER_MASK)
        mods.push('Super');

    // The key without Shift, so that Shift+1 stays "1".
    const [ok, base] = widget.get_display().translate_key(keycode, 0, controller.get_group());
    if (!ok)
        return null;
    let key = Gdk.keyval_name(Gdk.keyval_to_lower(base));
    if (key === 'ISO_Left_Tab')
        key = 'Tab';
    if (!key)
        return null;
    return {mods, key};
}

const MODIFIER_KEYS = /^(Control|Alt|Shift|Super|Meta|Hyper|ISO_Level\d|Caps_Lock|Num_Lock)/;

class Preferences {
    constructor(window) {
        this._window = window;
        this._pages = [];
        this._pending = new Map();
        this._chain = Promise.resolve();
        this.data = null;
        window.connect('close-request', () => {
            this._flush();
            return false;
        });
    }

    // --- talking to Sash -------------------------------------------------

    toast(text) {
        this._window.add_toast(new Adw.Toast({title: text, timeout: 6}));
    }

    // Runs requests one after another, so that changes arrive in the order they
    // were made. A failed request shows why and brings back what Sash really has.
    send(method, args, {reload = false} = {}) {
        this._chain = this._chain.then(async () => {
            try {
                const reply = await ask(method, args);
                if (reply.refused?.length) {
                    this.toast(`GNOME did not accept: ${reply.refused.join(', ')}. ` +
                        'Another shortcut is needed for the command.');
                }
                if (reload)
                    await this.reload();
            } catch (error) {
                this.toast(messageOf(error));
                await this.reload();
            }
        });
        return this._chain;
    }

    // Sends the newest value of a control that changes continuously, shortly
    // after it stopped changing.
    later(key, method, args) {
        const old = this._pending.get(key);
        if (old)
            GLib.source_remove(old.source);
        const source = GLib.timeout_add(GLib.PRIORITY_DEFAULT, DEBOUNCE_MS, () => {
            this._pending.delete(key);
            this.send(method, args);
            return GLib.SOURCE_REMOVE;
        });
        this._pending.set(key, {source, method, args});
    }

    // Window closing: nothing may wait for a timer any more.
    _flush() {
        for (const {source, method, args} of this._pending.values()) {
            GLib.source_remove(source);
            try {
                askSync(method, args);
            } catch (error) {
                console.error(`Sash: ${messageOf(error)}`);
            }
        }
        this._pending.clear();
    }

    setPreference(key, value) {
        return this.send('SetPreference', variant('(ss)', [key, JSON.stringify(value)]));
    }

    commit() {
        const strip = ({builtin: _builtin, ...command}) => command;
        const commands = {
            horizontal: this.commands.horizontal.map(strip),
            vertical: this.commands.vertical.map(strip),
        };
        return this.send('SetCommands', variant('(s)', [JSON.stringify(commands)]));
    }

    async reload() {
        let data = null;
        let failure = null;
        try {
            data = await ask('GetSettings');
        } catch (error) {
            failure = messageOf(error);
        }
        this._clearPages();
        if (!data) {
            this._add(this._offlinePage(failure));
            return;
        }
        this.data = data;
        this.commands = data.commands;
        this._add(this._generalPage());
        for (const axis of AXES)
            this._add(this._commandsPage(axis));
    }

    _add(page) {
        this._window.add(page);
        this._pages.push(page);
    }

    _clearPages() {
        for (const page of this._pages)
            this._window.remove(page);
        this._pages = [];
    }

    confirm(heading, body, label, action) {
        const dialog = new Adw.AlertDialog({heading, body});
        dialog.add_response('cancel', 'Cancel');
        dialog.add_response('ok', label);
        dialog.set_response_appearance('ok', Adw.ResponseAppearance.DESTRUCTIVE);
        dialog.set_close_response('cancel');
        dialog.set_default_response('cancel');
        dialog.connect('response', (_dialog, response) => {
            if (response === 'ok')
                action();
        });
        dialog.present(this._window);
    }

    // --- pages -----------------------------------------------------------

    _offlinePage(failure) {
        const page = new Adw.PreferencesPage({title: 'Sash', icon_name: 'preferences-system-symbolic'});
        const group = new Adw.PreferencesGroup();
        const retry = new Gtk.Button({label: 'Try Again', halign: Gtk.Align.CENTER});
        retry.add_css_class('suggested-action');
        retry.add_css_class('pill');
        retry.connect('clicked', () => this.reload());
        group.add(new Adw.StatusPage({
            icon_name: 'dialog-warning-symbolic',
            title: 'Sash Is Not Running',
            description: `Start sash, then try again.\n${failure ?? ''}`,
            child: retry,
        }));
        page.add(group);
        return page;
    }

    _switchRow(group, title, subtitle, key) {
        const row = new Adw.SwitchRow({title, subtitle, active: !!this.data.config[key]});
        row.connect('notify::active', () => this.setPreference(key, row.active));
        group.add(row);
        return row;
    }

    _spinRow(group, {title, subtitle, key, lower, upper, step, digits = 0, value}) {
        const row = new Adw.SpinRow({
            title,
            subtitle,
            digits,
            adjustment: new Gtk.Adjustment({lower, upper, step_increment: step, page_increment: step * 5, value}),
        });
        row.connect('notify::value', () => {
            const factor = 10 ** digits;
            this.later(key, 'SetPreference', variant('(ss)', [key, JSON.stringify(Math.round(row.value * factor) / factor)]));
        });
        group.add(row);
        return row;
    }

    _colorRow(group, title, subtitle, key, hex) {
        const row = new Adw.ActionRow({title, subtitle});
        const rgba = new Gdk.RGBA();
        rgba.parse(hex);
        const button = new Gtk.ColorDialogButton({
            dialog: new Gtk.ColorDialog({with_alpha: true, title}),
            rgba,
            valign: Gtk.Align.CENTER,
        });
        button.connect('notify::rgba', () => {
            const c = button.rgba;
            const byte = v => Math.round(v * 255).toString(16).padStart(2, '0');
            const value = `#${byte(c.red)}${byte(c.green)}${byte(c.blue)}${byte(c.alpha)}`;
            this.later(key, 'SetPreference', variant('(ss)', [key, JSON.stringify(value)]));
        });
        row.add_suffix(button);
        group.add(row);
    }

    _generalPage() {
        const config = this.data.config;
        const page = new Adw.PreferencesPage({title: 'General', icon_name: 'preferences-system-symbolic'});

        const dragging = new Adw.PreferencesGroup({title: 'Dragging'});
        this._switchRow(dragging, 'Snap Windows by Dragging',
            'Drag a window to a screen edge or corner to snap it.', 'snap_windows');
        this._switchRow(dragging, 'Restore Original Size When Dragging Away',
            'A snapped window gets its earlier size back when it is dragged off.', 'restore_to_original_size');
        page.add(dragging);

        const activation = new Adw.PreferencesGroup({title: 'Activation Areas'});
        this._spinRow(activation, {
            title: 'Edge Width', subtitle: 'Thickness in pixels of the strips along screen edges.',
            key: 'activation_edge_width', lower: 1, upper: 200, step: 1, value: config.activation_edge_width,
        });
        this._spinRow(activation, {
            title: 'Interior Area Scale', subtitle: 'Scale of activation areas that do not touch a screen edge.',
            key: 'activation_area_scale', lower: 0.1, upper: 3, step: 0.1, digits: 1,
            value: config.activation_area_scale,
        });
        page.add(activation);

        const spacing = new Adw.PreferencesGroup({title: 'Spacing'});
        this._spinRow(spacing, {
            title: 'Window Padding', subtitle: 'Margin in pixels around snapped windows.',
            key: 'padding', lower: 0, upper: 200, step: 1, value: config.padding,
        });
        this._switchRow(spacing, 'Fit Tightly to Screen Edges',
            'Leave out the margin along the edges of the screen.', 'fit_tightly_to_screen_edges');
        page.add(spacing);

        const preview = new Adw.PreferencesGroup({title: 'Snap Preview'});
        this._colorRow(preview, 'Fill', 'Shown over the area a dragged window would take.',
            'highlight.fill', config.highlight.fill);
        this._colorRow(preview, 'Outline', null, 'highlight.outline', config.highlight.outline);
        this._spinRow(preview, {
            title: 'Outline Width', subtitle: 'In pixels.',
            key: 'highlight.outline_width', lower: 1, upper: 20, step: 1, value: config.highlight.outline_width,
        });
        page.add(preview);

        const icon = new Adw.PreferencesGroup({title: 'Top Bar'});
        this._switchRow(icon, 'Hide the Sash Icon',
            'These settings stay reachable with: gnome-extensions prefs sash@sash.local', 'hide_menu_bar_icon');
        page.add(icon);

        page.add(this._ignoredGroup());

        const files = new Adw.PreferencesGroup({title: 'Files'});
        const open = new Adw.ActionRow({
            title: 'Open config.toml', subtitle: this.data.config_path ?? '', activatable: true,
        });
        open.add_suffix(new Gtk.Image({icon_name: 'document-edit-symbolic'}));
        open.connect('activated', () => this.send('OpenConfigFile', null));
        files.add(open);
        const restore = new Adw.ActionRow({title: 'Restore Preferences to Defaults', activatable: true});
        restore.add_css_class('error');
        restore.connect('activated', () => this.confirm(
            'Restore Preferences?', 'All preferences on this page return to their defaults. ' +
            'Commands and ignored applications are kept.', 'Restore',
            () => this.send('RestoreDefaults', variant('(s)', ['preferences']), {reload: true})));
        files.add(restore);
        page.add(files);
        return page;
    }

    _ignoredGroup() {
        const group = new Adw.PreferencesGroup({
            title: 'Ignored Applications',
            description: 'Sash leaves the windows of these applications alone. Use the window class, ' +
                'for example org.gnome.TextEditor. The menu\'s Ignore item adds to this list too.',
        });
        const fromPreferences = this.data.ignored.preferences;
        const fromMenu = this.data.ignored.menu;

        const rowFor = (app, subtitle, remove) => {
            const row = new Adw.ActionRow({title: app, subtitle});
            const button = Gtk.Button.new_from_icon_name('edit-delete-symbolic');
            button.add_css_class('flat');
            button.set_valign(Gtk.Align.CENTER);
            button.set_tooltip_text('Stop ignoring');
            button.connect('clicked', remove);
            row.add_suffix(button);
            return row;
        };
        for (const app of fromMenu) {
            group.add(rowFor(app, 'Added from the menu',
                () => this.send('SetIgnored', variant('(sb)', [app, false]), {reload: true})));
        }
        for (const app of fromPreferences) {
            group.add(rowFor(app, 'Listed in config.toml', () => this.send('SetPreference',
                variant('(ss)', ['ignored_apps', JSON.stringify(fromPreferences.filter(a => a !== app))]),
                {reload: true})));
        }

        const add = new Adw.EntryRow({title: 'Add an Application', show_apply_button: true});
        add.connect('apply', () => {
            const app = add.text.trim();
            if (app)
                this.send('SetIgnored', variant('(sb)', [app, true]), {reload: true});
        });
        group.add(add);
        return group;
    }

    _commandsPage(axis) {
        const page = new Adw.PreferencesPage({title: axis.title, icon_name: axis.icon});
        const group = new Adw.PreferencesGroup({title: `${axis.title} Commands`, description: axis.about});
        const addButton = new Gtk.Button({label: 'Add Command', valign: Gtk.Align.CENTER});
        addButton.add_css_class('suggested-action');
        addButton.connect('clicked', () => this._addCommand(axis));
        group.set_header_suffix(addButton);
        for (const command of this.commands[axis.key])
            group.add(this._commandRow(axis, command));
        page.add(group);

        const reset = new Adw.PreferencesGroup();
        const row = new Adw.ActionRow({title: 'Restore Default Commands', activatable: true,
            subtitle: 'Both orientations return to the built-in commands; added commands are removed.'});
        row.add_css_class('error');
        row.connect('activated', () => this.confirm(
            'Restore Default Commands?', 'Names, shortcuts, areas and menu entries of all commands ' +
            'go back to the built-in ones, and commands you added are removed.', 'Restore',
            () => this.send('RestoreDefaults', variant('(s)', ['commands']), {reload: true})));
        reset.add(row);
        page.add(reset);
        return page;
    }

    _addCommand(axis) {
        const taken = new Set(this.commands[axis.key].map(c => c.name.toLowerCase().replace(/[^a-z0-9]/g, '')));
        let name = 'Command';
        for (let n = 2; taken.has(name.toLowerCase().replace(/[^a-z0-9]/g, '')); n++)
            name = `Command ${n}`;
        this.commands[axis.key].push({
            name,
            target: [[axis.cols >> 2, axis.rows >> 2], [axis.cols >> 1, axis.rows >> 1]],
            special: null,
            activation: [],
            shortcut: null,
            separator_after: false,
            menu_bar: true,
            builtin: false,
        });
        this.commit().then(() => this.reload());
    }

    _commandRow(axis, command) {
        const row = new Adw.ExpanderRow({title: command.name});
        const describe = () => {
            const parts = [command.shortcut ?? 'No shortcut'];
            if (command.special)
                parts.push(SPECIALS[command.special] ?? command.special);
            row.subtitle = parts.join(' · ');
        };
        describe();

        if (!command.builtin) {
            const name = new Adw.EntryRow({title: 'Name', text: command.name, show_apply_button: true});
            name.connect('apply', () => {
                const text = name.text.trim();
                if (!text)
                    return;
                command.name = text;
                row.title = text;
                this.commit();
            });
            row.add_row(name);
        }

        row.add_row(this._shortcutRow(command, describe));

        const menu = new Adw.SwitchRow({title: 'Show in Menu', active: command.menu_bar});
        menu.connect('notify::active', () => {
            command.menu_bar = menu.active;
            this.commit();
        });
        row.add_row(menu);

        const divider = new Adw.SwitchRow({title: 'Divider After', subtitle: 'Draws a line below it in the menu.',
            active: command.separator_after});
        divider.connect('notify::active', () => {
            command.separator_after = divider.active;
            this.commit();
        });
        row.add_row(divider);

        if (command.target) {
            row.add_row(this._targetRow(axis, command));
            row.add_row(this._activationRow(axis, command));
        }

        if (!command.builtin) {
            const remove = new Adw.ActionRow({title: 'Delete Command', activatable: true});
            remove.add_css_class('error');
            remove.connect('activated', () => this.confirm(`Delete “${command.name}”?`,
                'The command and its shortcut are removed.', 'Delete', () => {
                    const list = this.commands[axis.key];
                    list.splice(list.indexOf(command), 1);
                    this.commit().then(() => this.reload());
                }));
            row.add_row(remove);
        }
        return row;
    }

    _shortcutRow(command, describe) {
        const row = new Adw.ActionRow({title: 'Shortcut'});
        const holder = new Gtk.Box({valign: Gtk.Align.CENTER, spacing: 6});
        const show = () => {
            clear(holder);
            if (command.shortcut) {
                holder.append(new Gtk.ShortcutLabel({accelerator: acceleratorOf(command.shortcut),
                    valign: Gtk.Align.CENTER}));
            } else {
                const none = new Gtk.Label({label: 'None'});
                none.add_css_class('dim-label');
                holder.append(none);
            }
        };
        show();

        const set = shortcut => {
            command.shortcut = shortcut;
            show();
            describe();
            this.commit();
        };
        const change = new Gtk.Button({label: 'Set…', valign: Gtk.Align.CENTER});
        change.connect('clicked', () => this._captureShortcut(set));
        const erase = Gtk.Button.new_from_icon_name('edit-clear-symbolic');
        erase.add_css_class('flat');
        erase.set_valign(Gtk.Align.CENTER);
        erase.set_tooltip_text('Remove the shortcut');
        erase.connect('clicked', () => set(null));
        row.add_suffix(holder);
        row.add_suffix(change);
        row.add_suffix(erase);
        return row;
    }

    _captureShortcut(done) {
        const dialog = new Adw.AlertDialog({
            heading: 'Press the New Shortcut',
            body: 'Hold the modifier keys and press a key. Backspace removes the shortcut, ' +
                'Esc cancels.',
        });
        dialog.add_response('cancel', 'Cancel');
        dialog.set_close_response('cancel');

        const keys = new Gtk.EventControllerKey({propagation_phase: Gtk.PropagationPhase.CAPTURE});
        keys.connect('key-pressed', (_controller, keyval, keycode, state) => {
            const name = Gdk.keyval_name(keyval) ?? '';
            if (MODIFIER_KEYS.test(name))
                return true;
            if (name === 'Escape')
                return false;
            if (name === 'BackSpace') {
                dialog.force_close();
                done(null);
                return true;
            }
            const shortcut = shortcutFromKey(dialog, keys, keycode, state);
            if (!shortcut)
                return true;
            if (shortcut.mods.length === 0 && !/^F\d+$/.test(shortcut.key)) {
                this.toast('Hold at least one modifier key (Ctrl, Alt, Shift or Super).');
                return true;
            }
            dialog.force_close();
            done([...shortcut.mods, shortcut.key].join('+'));
            return true;
        });
        dialog.add_controller(keys);
        dialog.present(this._window);
    }

    _pickerRow(title, subtitle) {
        const row = new Adw.PreferencesRow({activatable: false, title});
        const box = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 6,
            margin_top: 12, margin_bottom: 12, margin_start: 12, margin_end: 12});
        const heading = new Gtk.Label({label: title, xalign: 0});
        heading.add_css_class('heading');
        box.append(heading);
        if (subtitle) {
            const note = new Gtk.Label({label: subtitle, xalign: 0, wrap: true});
            note.add_css_class('dim-label');
            note.add_css_class('caption');
            box.append(note);
        }
        row.set_child(box);
        return {row, box};
    }

    _targetRow(axis, command) {
        const {row, box} = this._pickerRow('Target Area', 'Where the window goes. Drag over the grid.');
        const picker = new GridPicker({
            cols: axis.cols, rows: axis.rows, rect: toRect(command.target), color: null,
            onChange: rect => {
                command.target = toArray(rect);
                this.commit();
            },
        });
        box.append(picker.widget);
        return row;
    }

    _activationRow(axis, command) {
        const {row, box} = this._pickerRow('Activation Areas',
            'Dragging a window onto these parts of the screen snaps it to the target (the outlined area).');
        const list = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 10});
        box.append(list);

        const render = () => {
            clear(list);
            for (const area of command.activation) {
                const line = new Gtk.Box({spacing: 10});
                const picker = new GridPicker({
                    cols: axis.cols, rows: axis.rows, rect: toRect(area), context: toRect(command.target),
                    color: 'foreground',
                    onChange: rect => {
                        area[0] = [rect[0], rect[1]];
                        area[1] = [rect[2], rect[3]];
                        this.commit();
                    },
                });
                line.append(picker.widget);
                const remove = Gtk.Button.new_from_icon_name('edit-delete-symbolic');
                remove.add_css_class('flat');
                remove.set_valign(Gtk.Align.START);
                remove.set_tooltip_text('Remove this area');
                remove.connect('clicked', () => {
                    command.activation.splice(command.activation.indexOf(area), 1);
                    render();
                    this.commit();
                });
                line.append(remove);
                list.append(line);
            }
        };
        render();

        const add = new Gtk.Button({label: 'Add Activation Area', halign: Gtk.Align.START});
        add.connect('clicked', () => {
            command.activation.push([[0, 0], [1, 1]]);
            render();
            this.commit();
        });
        box.append(add);
        return row;
    }
}

export default class SashPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        window.set_default_size(760, 820);
        window._sashPreferences = new Preferences(window);
        return window._sashPreferences.reload();
    }
}
