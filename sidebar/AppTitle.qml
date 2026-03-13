import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Widgets
import QtQuick.Layouts

import "../theme"

Item {
    Layout.fillHeight: true
    Layout.preferredWidth: 36
    clip: true

    Item {
        anchors.centerIn: parent

        width: parent.height
        height: parent.width
        rotation: 270

        RowLayout {
            anchors.centerIn: parent
            spacing: 8
            width: Math.min(implicitWidth, parent.width - 20)
            IconImage {
                id: appIcon
                property string appClass: (Hyprland.activeToplevel?.wayland?.appId || "desktop").toLowerCase()
                source: Quickshell.iconPath(appClass, "application-x-executable")

                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter

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
                font.family: Theme.font
                font.pixelSize: 14

                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                elide: Text.ElideRight
            }
        }
    }
}
