// GNOME has no background layer for other programs to draw on, so apolline-wall opens an
// ordinary window per monitor instead: app id `apolline-wall`, and the monitor's connector
// after a colon in the title. This puts each one on its monitor, behind every window, on
// every workspace, and out of the overview and Alt+Tab.

import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Workspace} from 'resource:///org/gnome/shell/ui/workspace.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

const APP_ID = 'apolline-wall';

const ours = window => window?.get_wm_class?.() === APP_ID;

export default class ApollineWall extends Extension {
    enable() {
        this._adopted = new Map();
        this._injections = new InjectionManager();
        this._injections.overrideMethod(Workspace.prototype, '_isOverviewWindow', original =>
            function (window) {
                return !ours(window) && original.call(this, window);
            });
        this._injections.overrideMethod(Meta.Display.prototype, 'get_tab_list', original =>
            function (...args) {
                return original.apply(this, args).filter(window => !ours(window));
            });
        this._created = global.display.connect('window-created', (_, window) => this._watch(window));
        this._monitors = Main.layoutManager.connect('monitors-changed', () => {
            for (const window of this._adopted.keys())
                this._place(window);
        });
        for (const actor of global.get_window_actors())
            this._watch(actor.meta_window);
    }

    disable() {
        global.display.disconnect(this._created);
        Main.layoutManager.disconnect(this._monitors);
        for (const [window, ids] of this._adopted) {
            ids.forEach(id => window.disconnect(id));
            const actor = window.get_compositor_private();
            if (actor && actor.get_parent() !== global.window_group) {
                actor.get_parent()?.remove_child(actor);
                global.window_group.add_child(actor);
            }
        }
        this._adopted.clear();
        this._injections.clear();
        this._injections = null;
    }

    _watch(window) {
        if (ours(window)) {
            this._adopt(window);
            return;
        }
        // A Wayland window can name itself just after it appears.
        const id = window.connect('notify::wm-class', () => {
            window.disconnect(id);
            if (ours(window))
                this._adopt(window);
        });
    }

    _adopt(window) {
        if (this._adopted.has(window))
            return;
        this._adopted.set(window, [
            window.connect('raised', () => window.lower()),
            window.connect('notify::title', () => this._place(window)),
            window.connect('unmanaged', () => this._adopted.delete(window)),
        ]);
        window.stick();
        window.lower();
        this._place(window);
        this._sink(window);
    }

    // Under every window, just above GNOME's own background.
    _sink(window) {
        const actor = window.get_compositor_private();
        if (!actor) {
            GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                if (this._adopted.has(window))
                    this._sink(window);
                return GLib.SOURCE_REMOVE;
            });
            return;
        }
        const background = Main.layoutManager._backgroundGroup;
        if (actor.get_parent() === background)
            return;
        actor.get_parent()?.remove_child(actor);
        background.add_child(actor);
    }

    _place(window) {
        const connector = window.get_title()?.split(':')[1];
        if (!connector)
            return;
        const index = global.backend.get_monitor_manager().get_monitor_for_connector(connector);
        if (index < 0)
            return;
        const area = global.display.get_monitor_geometry(index);
        window.move_resize_frame(false, area.x, area.y, area.width, area.height);
    }
}
