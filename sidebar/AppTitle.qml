import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Widgets

import "../theme"

Row {
    spacing: 10
    rotation: 270
    width: 300

    IconImage {
        property string appClass: (Hyprland.activeToplevel?.wayland?.appId || "desktop").toLowerCase()
        source: Quickshell.iconPath(appClass, "application-x-executable")
        implicitSize: 16
        anchors.verticalCenter: parent.verticalCenter

        visible: appText.text !== "\uf4a9  Desktop"
    }

    Text {
        id: appText
        text: {
            let win = Hyprland.activeToplevel;
            if (win && win.workspace.id === Hyprland.focusedWorkspace.id) {
                return win.title;
            } else {
                return "\uf4a9  Desktop";
            }
        }
        color: Theme.text
        font.pixelSize: 14

        width: Math.min(implicitWidth, 274)
        elide: Text.ElideRight
        anchors.verticalCenter: parent.verticalCenter
    }
}
