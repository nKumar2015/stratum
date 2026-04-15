import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    WlSessionLock {
        WlSessionLockSurface {
            id: surface
            Component.onCompleted: {
                console.log("Surface created");
                // Check for 'output' property
                if (surface.output !== undefined) {
                    console.log("Surface has 'output' property");
                } else {
                    console.log("Surface DOES NOT have 'output' property");
                }
                
                // Print all properties to be sure
                for (let prop in surface) {
                    if (prop === "output" || prop === "screen") {
                        console.log("Prop " + prop + " exists");
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
