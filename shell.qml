import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts

PanelWindow {
    anchors.top: true
    anchors.left: true
    anchors.bottom: true

    implicitWidth: 40

    color: "#1A1B26"

    Text {
        anchors.centerIn: parent
        text: "Current Application"
        color: "#A9B1D6"
        font.pixelSize: 14
        rotation: 270
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8

        Repeater {
            model: 9

            Text {
                property var ws: Hyprland.workspaces.values.find(w => w.id === index + 1)
                property bool isActive: Hyprland.focusedWorkspace?.id === (index + 1)

                Layout.alignment: Qt.AlignHCenter

                text: index + 1
                color: isActive ? "#0DB9D7" : (ws ? "#7AA2F7" : "#444B6A")
                font {
                    pixelSize: 14
                    bold: true
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: Hyprland.dispatch("workspace " + (index + 1))
                }
            }
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
