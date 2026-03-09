import Quickshell.Hyprland
import QtQuick
import "../theme"

Item {
    id: root

    property int itemHeight: 20
    property int itemSpacing: 3

    implicitWidth: 40
    implicitHeight: 110
    clip: true

    property int activeWsId: Hyprland.focusedWorkspace?.id || 1
    property int startIdx: Math.min(4, Math.max(0, activeWsId - 3))

    Column {
        id: mainColumn
        width: 30
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: root.itemSpacing

        // 3. The Slide Animation
        // When startIdx changes, this Y position will glide to the new spot
        y: -(root.startIdx * (root.itemHeight + root.itemSpacing))

        Behavior on y {
            NumberAnimation {
                duration: 400
                easing.type: Easing.OutBack // Gives that nice "mechanical" bounce
            }
        }

        Repeater {
            model: 9 // All workspaces exist in this long column

            Text {
                property int wsId: index + 1
                property var ws: Hyprland.workspaces.values.find(w => w.id === wsId)
                property bool isActive: Hyprland.focusedWorkspace?.id === wsId

                height: 20
                anchors.horizontalCenter: parent.horizontalCenter

                text: wsId
                color: isActive ? Theme.activeWs : (ws ? Theme.defaultWs : Theme.inactiveWs)
                font {
                    pixelSize: 14
                    bold: true
                }

                Behavior on color {
                    ColorAnimation {
                        duration: 200
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: Hyprland.dispatch("workspace " + wsId)
                    onWheel: wheel => {
                        if (wheel.angleDelta.y > 0) {
                            Hyprland.dispatch("workspace " + Math.max(1, root.activeWsId - 1));
                        } else {
                            Hyprland.dispatch("workspace " + Math.min(9, root.activeWsId + 1));
                        }
                    }
                }
            }
        }
    }
}
