import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                console.log("Surface created");
                console.log("Surface screen: " + surface.screen);
                // Try to set it
                try {
                   surface.screen = Quickshell.screens[0];
                   console.log("Set screen successfully");
                } catch (e) {
                   console.log("Failed to set screen: " + e);
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
