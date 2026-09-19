import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// The game reports its window as "SlayTheSpire2" / "Slay the Spire 2"; nothing else in this
// session matches, so Steam's own windows are left alone.
const MATCH = /spire/i;

// The window is mapped well before the game settles: it resizes itself twice on the way in and
// its saved display settings are applied later still.
const ATTEMPT_DELAYS = [500, 1500, 3000, 6000, 12000];

// Long enough for the game's own startup pass to finish before answering it, short enough that
// the shell is never left drawn over the game. Also debounces the change our own call causes.
const REASSERT_DELAY = 400;

// The backstop. notify::fullscreen only fires on a transition, so a window that never became
// fullscreen in the first place - because the early attempts landed before it was established -
// produces no signal at all and would otherwise sit windowed forever. That is exactly what
// happened after an episode relaunch: the window stayed 718x552 until the extension was
// re-enabled by hand. Sweeping catches it without needing a transition to react to.
const SWEEP_SECONDS = 5;

// The game puts itself back into windowed mode while it applies its saved display settings. A
// bounded number of answers covers that without spinning forever if a build ever refuses. The
// count is per window, so a fresh game every episode starts with a fresh budget.
const MAX_REASSERTS = 40;

export default class JevFullscreen extends Extension {
    enable() {
        this._timeouts = new Set();
        this._watched = new Map();
        this._reasserts = new Map();
        this._pending = 0;
        this._createdId = global.display.connect('window-created',
            (_display, window) => this._consider(window));
        // Anything that takes focus - an update-notifier dialog, a Steam popup - leaves the game
        // fullscreen but unfocused, and Mutter then draws the top bar and the dock over it.
        this._focusId = global.display.connect('notify::focus-window', () => this._schedule());
        this._sweepId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, SWEEP_SECONDS, () => {
            const game = this._gameWindow();
            if (game && (!game.is_fullscreen() || global.display.focus_window !== game))
                this._apply(game);
            return GLib.SOURCE_CONTINUE;
        });
        for (const actor of global.get_window_actors())
            this._consider(actor.meta_window);
        this._dump('enable');
    }

    disable() {
        for (const id of [this._createdId, this._focusId]) {
            if (id)
                global.display.disconnect(id);
        }
        this._createdId = 0;
        this._focusId = 0;
        for (const [window, ids] of this._watched) {
            for (const id of ids) {
                try {
                    window.disconnect(id);
                } catch (error) {
                    // Already unmanaged; nothing to release.
                }
            }
        }
        this._watched.clear();
        this._reasserts.clear();
        for (const id of [this._sweepId, this._pending]) {
            if (id)
                GLib.source_remove(id);
        }
        this._sweepId = 0;
        this._pending = 0;
        for (const id of this._timeouts)
            GLib.source_remove(id);
        this._timeouts.clear();
    }

    _isGame(window) {
        if (!window || window.window_type !== Meta.WindowType.NORMAL)
            return false;
        return MATCH.test(`${window.get_wm_class() ?? ''} ${window.get_title() ?? ''}`);
    }

    _gameWindow() {
        for (const actor of global.get_window_actors()) {
            if (this._isGame(actor.meta_window))
                return actor.meta_window;
        }
        return null;
    }

    _consider(window) {
        if (!window || window.window_type !== Meta.WindowType.NORMAL)
            return;
        for (const delay of ATTEMPT_DELAYS) {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
                this._timeouts.delete(id);
                this._watch(window);
                this._apply(window);
                return GLib.SOURCE_REMOVE;
            });
            this._timeouts.add(id);
        }
    }

    _watch(window) {
        if (!this._isGame(window) || this._watched.has(window))
            return;
        const ids = [
            window.connect('notify::fullscreen', () => this._schedule()),
            window.connect('unmanaged', () => this._unwatch(window)),
        ];
        this._watched.set(window, ids);
    }

    _unwatch(window) {
        const ids = this._watched.get(window);
        if (ids) {
            for (const id of ids) {
                try {
                    window.disconnect(id);
                } catch (error) {
                    // Already unmanaged.
                }
            }
            this._watched.delete(window);
        }
        this._reasserts.delete(window);
    }

    _schedule() {
        if (this._pending)
            return;
        this._pending = GLib.timeout_add(GLib.PRIORITY_DEFAULT, REASSERT_DELAY, () => {
            this._pending = 0;
            const game = this._gameWindow();
            if (game)
                this._apply(game);
            return GLib.SOURCE_REMOVE;
        });
    }

    _apply(window) {
        try {
            // A window destroyed between the timeout being armed and firing has no actor.
            if (!window || !window.get_compositor_private() || !this._isGame(window))
                return;
            this._watch(window);
            if (!window.is_fullscreen()) {
                const used = this._reasserts.get(window) ?? 0;
                if (used >= MAX_REASSERTS) {
                    if (used === MAX_REASSERTS) {
                        this._reasserts.set(window, used + 1);
                        log(`jev-fullscreen: giving up after ${MAX_REASSERTS} attempts the game undid`);
                    }
                    return;
                }
                this._reasserts.set(window, used + 1);
                window.make_fullscreen();
            }
            // Mutter only hides the top bar and the dock for the *focused* fullscreen window.
            if (global.display.focus_window !== window) {
                window.activate(global.get_current_time());
                window.raise();
            }
        } catch (error) {
            logError(error, 'jev-fullscreen');
        }
    }

    _dump(when) {
        try {
            const focus = global.display.focus_window;
            const rows = global.get_window_actors().map(actor => {
                const w = actor.meta_window;
                const r = w.get_frame_rect();
                return `[${w.get_wm_class() ?? '?'}|${w.get_title() ?? '?'}`
                    + `|fs=${w.is_fullscreen()}|focus=${w === focus}`
                    + `|${r.width}x${r.height}+${r.x}+${r.y}]`;
            });
            log(`jev-fullscreen ${when}: ${rows.length} windows ${rows.join(' ')}`);
        } catch (error) {
            logError(error, 'jev-fullscreen dump');
        }
    }
}
