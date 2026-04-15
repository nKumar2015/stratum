import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                console.log("Surface created");
                try {
                    surface.output = Quickshell.screens[0];
                    console.log("Set output property successfully");
                } catch (e) {
                    console.log("Failed to set output property: " + e);
                }
            }
        }
    }
}
