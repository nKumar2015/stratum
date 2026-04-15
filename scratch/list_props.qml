import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                console.log("SURFACE_PROPS_START");
                for (let prop in surface) {
                    console.log("PROP: " + prop);
                }
                console.log("SURFACE_PROPS_END");
            }
        }
    }
    Timer {
        interval: 1000
        running: true
        onTriggered: Qt.quit()
    }
}
