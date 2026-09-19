import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// The game reports its window as "SlayTheSpire2" / "Slay the Spire 2"; nothing else in this
// session matches, so Steam's own windows are left alone.
const MATCH = /spire/i;

// The window is mapped before Godot settles its size, and the launcher resizes it once more
// on the way in. Re-check for a while rather than acting on the first map only.
const ATTEMPT_DELAYS = [500, 1500, 3000, 6000, 12000];

export default class JevFullscreen extends Extension {
    enable() {
        this._timeouts = new Set();
        this._createdId = global.display.connect('window-created',
            (_display, window) => this._consider(window));
        for (const actor of global.get_window_actors())
            this._consider(actor.meta_window);
    }

    disable() {
        if (this._createdId) {
            global.display.disconnect(this._createdId);
            this._createdId = null;
        }
        for (const id of this._timeouts)
            GLib.source_remove(id);
        this._timeouts.clear();
    }

    _consider(window) {
        if (!window || window.window_type !== Meta.WindowType.NORMAL)
            return;
        for (const delay of ATTEMPT_DELAYS) {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
                this._timeouts.delete(id);
                this._apply(window);
                return GLib.SOURCE_REMOVE;
            });
            this._timeouts.add(id);
        }
    }

    _apply(window) {
        try {
            // A window destroyed between the timeout being armed and firing has no actor.
            if (!window || !window.get_compositor_private())
                return;
            const name = `${window.get_wm_class() ?? ''} ${window.get_title() ?? ''}`;
            if (!MATCH.test(name))
                return;
            if (!window.is_fullscreen())
                window.make_fullscreen();
            // Mutter only hides the top bar and the dock for the *focused* fullscreen window.
            // Nothing in this session ever clicks the game, so it is fullscreen but unfocused and
            // the shell keeps drawing over it. Raise and focus it here.
            if (!window.has_focus()) {
                window.activate(global.get_current_time());
                window.raise();
            }
        } catch (error) {
            logError(error, 'jev-fullscreen');
        }
    }
}
