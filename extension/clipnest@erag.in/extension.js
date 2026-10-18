// Clipnest helper: answers Clipnest's question "where is the user typing?".
//
// Wayland doesn't let apps see other apps' text cursors or the mouse pointer,
// but GNOME Shell knows both: input methods report the text cursor to the
// shell's IBus candidate popup, which keeps it as an invisible "dummy cursor"
// actor. This exports that position (and the pointer) on the session bus.

import Gio from 'gi://Gio';
import {getIBusManager} from 'resource:///org/gnome/shell/misc/ibusManager.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'in.erag.Clipnest.Shell';
const OBJECT_PATH = '/in/erag/Clipnest/Shell';
const IFACE = `
<node>
  <interface name="in.erag.Clipnest.Shell">
    <method name="GetCaret">
      <arg type="b" direction="out" name="found"/>
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
      <arg type="i" direction="out" name="width"/>
      <arg type="i" direction="out" name="height"/>
    </method>
    <method name="GetPointer">
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
    </method>
  </interface>
</node>`;

class Helper {
    // The text cursor of the focused window, if an input method reported one.
    GetCaret() {
        try {
            const cursor = getIBusManager()._candidatePopup._dummyCursor;
            const [x, y] = cursor.get_position();
            const [width, height] = cursor.get_size();
            const focus = global.display.focus_window;
            if (!focus || (x === 0 && y === 0 && width === 0 && height === 0))
                return [false, 0, 0, 0, 0];
            // Only trust it inside the focused window: an app that doesn't
            // report its cursor leaves the previous app's position behind.
            const frame = focus.get_frame_rect();
            const inside = x >= frame.x && x <= frame.x + frame.width &&
                           y >= frame.y && y <= frame.y + frame.height;
            if (!inside)
                return [false, 0, 0, 0, 0];
            return [true, Math.round(x), Math.round(y), Math.round(width), Math.round(height)];
        } catch (e) {
            return [false, 0, 0, 0, 0];
        }
    }

    GetPointer() {
        const [x, y] = global.get_pointer();
        return [x, y];
    }
}

export default class ClipnestHelper extends Extension {
    enable() {
        this._exported = Gio.DBusExportedObject.wrapJSObject(IFACE, new Helper());
        this._exported.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME,
            Gio.BusNameOwnerFlags.NONE, null, null, null);
    }

    disable() {
        Gio.bus_unown_name(this._nameId);
        this._exported.unexport();
        this._exported = null;
    }
}
