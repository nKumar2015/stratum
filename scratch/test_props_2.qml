import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                console.log("Surface created");
                // Check if Screen is available
                for (let prop in surface) {
                    if (prop.toLowerCase().indexOf("screen") !== -1 || prop.toLowerCase().indexOf("output") !== -1) {
                        console.log("Found prop: " + prop);
                    }
                }
            }
        }
    }
    Timer {
        interval: 1000
        running: true
        onTriggered: Qt.quit()
    }
}
