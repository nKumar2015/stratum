import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                if (surface.output !== undefined) {
                    console.log("FOUND_OUTPUT_PROP");
                }
                if (surface.screen !== undefined) {
                    console.log("FOUND_SCREEN_PROP");
                }
            }
        }
    }
}
