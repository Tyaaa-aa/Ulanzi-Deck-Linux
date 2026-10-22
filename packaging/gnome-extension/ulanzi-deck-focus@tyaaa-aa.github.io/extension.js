// Ulanzi Deck: tell the deck service which app is in front (for auto profile switching).
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

export default class UlanziDeckFocus extends Extension {
    enable() {
        this._id = global.display.connect('notify::focus-window', () => this._report());
        this._report();
    }

    disable() {
        if (this._id)
            global.display.disconnect(this._id);
        this._id = null;
    }

    _report() {
        const w = global.display.focus_window;
        if (!w)
            return;
        const app = Shell.WindowTracker.get_default().get_window_app(w);
        const id = app ? (app.get_id() || '').replace(/\.desktop$/, '') : '';
        Gio.DBus.session.call(
            'io.github.tyaaa_aa.UlanziDeck.Service',
            '/io/github/tyaaa_aa/UlanziDeck',
            'io.github.tyaaa_aa.UlanziDeck1',
            'WindowActivated',
            new GLib.Variant('(sss)', [w.get_wm_class() || '', id, w.get_title() || '']),
            null, Gio.DBusCallFlags.NO_AUTO_START, -1, null, null);
    }
}
