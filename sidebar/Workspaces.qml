pragma ComponentBehavior: Bound
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts
import "../theme"

Item {
    id: wsRoot

    required property var monitor

    property int itemHeight: 20
    property int itemSpacing: 3

    implicitWidth: 40
    implicitHeight: 115
    clip: true

    readonly property var workspaceIds: {
        const ids = [];
        const targetMonitor = wsRoot.monitor;
        const allWorkspaces = Hyprland.workspaces.values || [];

        for (let index = 0; index < allWorkspaces.length; index++) {
            const workspace = allWorkspaces[index];
            if (!workspace || workspace.id <= 0)
                continue;
            if (workspace.monitor !== targetMonitor)
                continue;
            ids.push(workspace.id);
        }

        ids.sort((left, right) => left - right);

        const activeWorkspaceId = targetMonitor?.activeWorkspace?.id || -1;
        if (activeWorkspaceId > 0 && ids.indexOf(activeWorkspaceId) === -1)
            ids.push(activeWorkspaceId);

        ids.sort((left, right) => left - right);
        return ids;
    }
    property int activeWsId: monitor?.activeWorkspace?.id || workspaceIds[0] || 1
    property int startIdx: {
        const ids = workspaceIds;
        const activeIndex = Math.max(0, ids.indexOf(activeWsId));
        return Math.max(0, Math.min(Math.max(0, ids.length - 5), activeIndex - 2));
    }

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
            model: wsRoot.workspaceIds

            Text {
                id: wsText
                required property var modelData
                property int wsId: Number(modelData)
                property var ws: Hyprland.workspaces.values.find(w => w.id === wsId)
                property bool isActive: wsRoot.monitor?.activeWorkspace?.id === wsId
                property bool hasOpenWindows: (ws?.toplevels?.values?.length || 0) > 0
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredHeight: 20

                text: wsId
                color: isActive ? Theme.activeWs : (hasOpenWindows ? Theme.defaultWs : Theme.inactiveWs)
                font {
                    pixelSize: 15
                    bold: true
                    family: Theme.font
                }

                Rectangle {
                    visible: wsText.isActive
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.bottom: parent.bottom
                    width: 14
                    height: 2
                    radius: 2
                    color: Theme.activeWs
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
                        const ids = wsRoot.workspaceIds;
                        const currentIndex = ids.indexOf(wsRoot.activeWsId);
                        if (currentIndex < 0)
                            return;

                        if (wheel.angleDelta.y > 0) {
                            Hyprland.dispatch("workspace " + ids[Math.max(0, currentIndex - 1)]);
                        } else {
                            Hyprland.dispatch("workspace " + ids[Math.min(ids.length - 1, currentIndex + 1)]);
                        }
                    }
                }
            }
        }
    }
}
