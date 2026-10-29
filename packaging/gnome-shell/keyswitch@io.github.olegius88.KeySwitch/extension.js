import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {getKeyboardManager} from 'resource:///org/gnome/shell/misc/keyboardManager.js';
import {getInputSourceManager} from 'resource:///org/gnome/shell/ui/status/keyboard.js';

// While a modal of the Shell itself has the keyboard (a polkit password prompt, Alt+F2, the
// overview), the keys go to the Shell, not to the focused window: it is reported as a window of
// its own, so KeySwitch leaves the word typed elsewhere and reads the Shell's field (a password
// prompt is a password field) instead of the window behind it.
const SHELL_WINDOW_ID = 0x7fffffff;

const IFACE_XML = `
<node>
  <interface name="io.github.olegius88.KeySwitch.Gnome">
    <method name="GetState">
      <arg type="s" direction="out" name="state"/>
    </method>
    <method name="ActivateLayout">
      <arg type="u" direction="in" name="index"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="MoveWindow">
      <arg type="u" direction="in" name="pid"/>
      <arg type="s" direction="in" name="title"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="ActivateWindow">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <signal name="StateChanged">
      <arg type="s" name="state"/>
    </signal>
  </interface>
</node>`;

export default class KeySwitchExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE_XML, this);
        this._dbus.export(Gio.DBus.session, '/io/github/olegius88/KeySwitch/Gnome');
        this._ism = getInputSourceManager();
        this._keyboardManager = getKeyboardManager();
        this._inputSettings = new Gio.Settings({
            schema_id: 'org.gnome.desktop.input-sources',
        });
        // The input source changes at once, the keymap group a moment later (set_keymap_async): a
        // layout counts only once mutter has applied it and told the focused client, so the
        // group is the one mutter announced, read in its own signals. A group change alone comes
        // with its index; a keymap made anew (the first switch after login, when the keymap in
        // place is not the Shell's) comes as keymap-changed, with the group the Shell asked for.
        // GNOME 50.1 has no getter for the applied group (Clutter.Keymap.get_layout_index comes
        // later): at the start, the source in place is taken as applied.
        this._appliedGroup = this._targetGroup();
        this._groupSig = global.backend.connect('keymap-layout-group-changed', (_backend, index) => {
            this._appliedGroup = index;
            this._emitState();
        });
        this._keymapSig = global.backend.connect('keymap-changed', () => {
            const keymap = global.stage.context.get_backend().get_default_seat().get_keymap();
            this._appliedGroup = typeof keymap.get_layout_index === 'function'
                ? keymap.get_layout_index() : this._targetGroup();
            this._emitState();
        });
        this._layoutSig = this._ism.connect('current-source-changed', () => {
            this._emitState();
        });
        this._sourcesSig = this._ism.connect('sources-changed', () => {
            this._emitState();
        });
        this._focusSig = global.display.connect('notify::focus-window', () => {
            this._emitState();
        });
        this._stageFocusSig = global.stage.connect('notify::key-focus', () => {
            this._emitState();
        });
        // What the focused client tells the input method of its field (text-input): a password
        // field says so here at once, where an AT-SPI read may come too late.
        this._purposeSigs = Main.inputMethod ? [
            Main.inputMethod.connect('notify::content-purpose', () => this._emitState()),
            Main.inputMethod.connect('notify::content-hints', () => this._emitState()),
        ] : [];
        this._shellPid = new Gio.Credentials().get_unix_pid();
        this._shieldSig = 0;
        if (Main.screenShield) {
            this._shieldSig = Main.screenShield.connect('locked-changed', () => {
                this._emitState();
            });
        }
    }

    disable() {
        if (this._groupSig) {
            global.backend.disconnect(this._groupSig);
            this._groupSig = 0;
        }
        if (this._keymapSig) {
            global.backend.disconnect(this._keymapSig);
            this._keymapSig = 0;
        }
        if (this._layoutSig && this._ism) {
            this._ism.disconnect(this._layoutSig);
            this._layoutSig = 0;
        }
        if (this._sourcesSig && this._ism) {
            this._ism.disconnect(this._sourcesSig);
            this._sourcesSig = 0;
        }
        if (this._focusSig) {
            global.display.disconnect(this._focusSig);
            this._focusSig = 0;
        }
        if (this._stageFocusSig) {
            global.stage.disconnect(this._stageFocusSig);
            this._stageFocusSig = 0;
        }
        for (const id of this._purposeSigs || [])
            Main.inputMethod.disconnect(id);
        this._purposeSigs = [];
        if (this._shieldSig && Main.screenShield) {
            Main.screenShield.disconnect(this._shieldSig);
            this._shieldSig = 0;
        }
        if (this._dbus) {
            this._dbus.unexport();
            this._dbus = null;
        }
        this._inputSettings = null;
        this._keyboardManager = null;
        this._ism = null;
    }

    _targetGroup() {
        // The keymap group the keyboard manager puts the current source in (its layouts are packed
        // into keymaps of a few groups each).
        const source = this._ism ? this._ism.currentSource : null;
        const infos = this._keyboardManager ? this._keyboardManager._layoutInfos : null;
        const info = source && infos ? infos[source.xkbId] : null;
        return info ? info.groupIndex : -1;
    }

    _clientPasswordFocus() {
        const im = Main.inputMethod;
        // The purpose stays after the field loses the keyboard: only a field that has it counts.
        if (!im || im._currentFocus === null)
            return false;
        const hidden = Clutter.InputContentHintFlags.SENSITIVE_DATA | Clutter.InputContentHintFlags.HIDDEN_TEXT;
        return im.content_purpose === Clutter.InputContentPurpose.PASSWORD || (im.content_hints & hidden) !== 0;
    }

    _appliedLayoutIndex() {
        // -1 while the current source is still being applied: the caller waits for the next state.
        const source = this._ism ? this._ism.currentSource : null;
        const target = this._targetGroup();
        if (!source || target < 0 || target !== this._appliedGroup)
            return -1;
        return Number(source.index || 0);
    }

    _emitState() {
        if (!this._dbus)
            return;
        const payload = this._buildStateJson();
        this._dbus.emit_signal('StateChanged', new GLib.Variant('(s)', [payload]));
    }

    _findWindowById(id) {
        for (const actor of global.get_window_actors()) {
            const win = actor.get_meta_window();
            if (win && win.get_stable_sequence() === id)
                return win;
        }
        return null;
    }

    _findWindowByPid(pid, title) {
        let fallback = null;
        const focused = global.display.focus_window;
        if (focused && focused.get_pid() === pid) {
            if (!title || (focused.get_title() || '') === title)
                return focused;
            fallback = focused;
        }
        for (const actor of global.get_window_actors()) {
            const win = actor.get_meta_window();
            if (!win || win.get_pid() !== pid)
                continue;
            if (!title || (win.get_title() || '') === title)
                return win;
            if (!fallback)
                fallback = win;
        }
        return fallback;
    }

    _buildStateJson() {
        const layouts = [];
        if (this._ism && this._ism.inputSources) {
            const sources = this._ism.inputSources;
            const keys = Object.keys(sources)
                .map(k => parseInt(k, 10))
                .filter(k => !Number.isNaN(k))
                .sort((a, b) => a - b);
            for (const k of keys) {
                const src = sources[k];
                if (src && src.id)
                    layouts.push(String(src.id));
            }
        }
        const current = this._appliedLayoutIndex();

        let winObj = null;
        const w = global.display.focus_window;
        if (Main.modalCount > 0) {
            // The Shell's entries are not readable over AT-SPI: whether the one with the keyboard
            // hides what is typed (a password prompt) is said here.
            const focus = global.stage.get_key_focus();
            const text = focus instanceof Clutter.Text ? focus : (focus && focus.clutter_text) || null;
            // A gunichar comes to JavaScript as a string: no password character is "" or "\0".
            const hiddenWith = text ? text.get_password_char() : '';
            winObj = {
                id: SHELL_WINDOW_ID,
                wm_class: 'gnome-shell',
                wm_class_instance: 'gnome-shell',
                app_id: 'org.gnome.Shell',
                pid: this._shellPid,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                password: Boolean(hiddenWith) && hiddenWith !== '\0',
            };
        } else if (w) {
            const tracker = Shell.WindowTracker.get_default();
            const app = tracker ? tracker.get_window_app(w) : null;
            const appId = (app ? app.get_id() : '') || w.get_gtk_application_id() || '';
            // The client area: a frame mutter draws around a window is not part of it, and the
            // positions an application reports (the caret over AT-SPI) are relative to it.
            const rect = w.frame_rect_to_client_rect(w.get_frame_rect());
            winObj = {
                id: Number(w.get_stable_sequence() || 0),
                wm_class: String(w.get_wm_class() || ''),
                wm_class_instance: String(w.get_wm_class_instance() || ''),
                app_id: String(appId || ''),
                pid: Number(w.get_pid() || 0),
                x: Math.round(rect.x),
                y: Math.round(rect.y),
                width: Math.round(rect.width),
                height: Math.round(rect.height),
                password: this._clientPasswordFocus(),
            };
        }

        const [px, py] = global.get_pointer();
        const locked = Boolean(
            (Main.sessionMode && Main.sessionMode.isLocked) ||
            (Main.screenShield && Main.screenShield.locked)
        );
        const perWindow = Boolean(
            this._inputSettings && this._inputSettings.get_boolean('per-window')
        );

        return JSON.stringify({
            layouts,
            current_layout: current,
            window: winObj,
            pointer: {x: Math.round(px), y: Math.round(py)},
            locked,
            per_window_layout: perWindow,
        });
    }

    GetState() {
        return this._buildStateJson();
    }

    ActivateLayout(index) {
        if (!this._ism || !this._ism.inputSources)
            return false;
        const src = this._ism.inputSources[index];
        if (!src)
            return false;
        src.activate(true);
        return true;
    }

    MoveWindow(pid, title, x, y) {
        const w = this._findWindowByPid(pid, title);
        if (!w)
            return false;
        w.move_frame(true, x, y);
        return true;
    }

    ActivateWindow(id) {
        const w = this._findWindowById(id);
        if (!w)
            return false;
        Main.activateWindow(w);
        return true;
    }
}
