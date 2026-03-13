pragma ComponentBehavior: Bound
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts
import "../theme"

Item {
    id: wsRoot

    property int itemHeight: 20
    property int itemSpacing: 3

    implicitWidth: 40
    implicitHeight: 110
    clip: true

    property int activeWsId: Hyprland.focusedWorkspace?.id || 1
    property int startIdx: Math.min(4, Math.max(0, activeWsId - 3))

    ColumnLayout {
        id: mainColumn
        spacing: wsRoot.itemSpacing
        anchors.left: parent.left
        anchors.right: parent.right

        y: -(wsRoot.startIdx * (wsRoot.itemHeight + wsRoot.itemSpacing))

        Behavior on y {
            NumberAnimation {
                duration: 400
                easing.type: Easing.OutBack // Gives that nice "mechanical" bounce
            }
        }

        Repeater {
            model: 9 // All workspaces exist in this long column

            Text {
                id: wsText
                required property int index
                property int wsId: index + 1
                property var ws: Hyprland.workspaces.values.find(w => w.id === wsId)
                property bool isActive: Hyprland.focusedWorkspace?.id === wsId
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredHeight: 20

                text: wsId
                color: isActive ? Theme.activeWs : (ws ? Theme.defaultWs : Theme.inactiveWs)
                font {
                    pixelSize: 15
                    bold: true
                    family: Theme.font
                }

                Behavior on color {
                    ColorAnimation {
                        duration: 200
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: Hyprland.dispatch("workspace " + wsText.wsId)
                    onWheel: wheel => {
                        if (wheel.angleDelta.y > 0) {
                            Hyprland.dispatch("workspace " + Math.max(1, wsRoot.activeWsId - 1));
                        } else {
                            Hyprland.dispatch("workspace " + Math.min(9, wsRoot.activeWsId + 1));
                        }
                    }
                }
            }
        }
    }
}
