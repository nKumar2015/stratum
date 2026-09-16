pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Quickshell.Io

import "../globals"

Item {
    id: wsRoot

    required property string monitorName

    property int itemHeight: 20
    property int itemSpacing: 3
    property int wsTarget: 0
    implicitWidth: 40
    implicitHeight: 115
    clip: true

    property int cliActiveWorkspaceId: -1
    property var cliOccupiedWorkspaceIds: []

    function parseMonitors(text) {
        try {
            const monitors = JSON.parse(text);
            for (let index = 0; index < monitors.length; index++) {
                if (monitors[index].name === wsRoot.monitorName) {
                    const activeWorkspace = monitors[index].activeWorkspace || {};
                    wsRoot.cliActiveWorkspaceId = activeWorkspace.id || Number(activeWorkspace.name) || -1;
                    return;
                }
            }
            wsRoot.cliActiveWorkspaceId = -1;
        } catch (error) {
            console.warn("[Workspaces] Unable to parse hyprctl monitors:", error);
        }
    }

    function parseClients(text) {
        try {
            const clients = JSON.parse(text);
            const ids = [];
            for (let index = 0; index < clients.length; index++) {
                const workspace = clients[index].workspace || {};
                const workspaceId = workspace.id || Number(workspace.name) || -1;
                if (workspaceId > 0 && ids.indexOf(workspaceId) === -1)
                    ids.push(workspaceId);
            }
            wsRoot.cliOccupiedWorkspaceIds = ids;
        } catch (error) {
            console.warn("[Workspaces] Unable to parse hyprctl clients:", error);
        }
    }

    Process {
        id: monitorProcess
        command: ["hyprctl", "monitors", "-j"]
        running: true
        onRunningChanged: if (!running) running = true
        stdout: StdioCollector {
            onStreamFinished: wsRoot.parseMonitors(text)
        }
    }

    Process {
        id: clientsProcess
        command: ["hyprctl", "clients", "-j"]
        running: true
        onRunningChanged: if (!running) running = true
        stdout: StdioCollector {
            onStreamFinished: wsRoot.parseClients(text)
        }
    }

    Process {
        id: dispatchProcess
    }

    readonly property var workspaceIds: {
        const activeWorkspaceId = wsRoot.cliActiveWorkspaceId;
        if (activeWorkspaceId <= 0)
            return [];

        const rangeStart = Math.floor((activeWorkspaceId - 1) / 10) * 10 + 1;
        const ids = [];
        for (let offset = 0; offset < 10; offset++)
            ids.push(rangeStart + offset);

        return ids;
    }
    property int activeWsId: cliActiveWorkspaceId || workspaceIds[0] || 1
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
                property bool isActive: wsRoot.activeWsId === wsId
                property bool hasOpenWindows: wsRoot.cliOccupiedWorkspaceIds.indexOf(wsId) !== -1
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredHeight: 20

                text: wsId
                color: isActive ? Theme.palette.primary : (hasOpenWindows ? Theme.palette.secondary : Theme.palette.textMuted)
                font {
                    pixelSize: 15
                    bold: true
                    family: Theme.palette.font
                }

                Rectangle {
                    visible: wsText.isActive
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.bottom: parent.bottom
                    width: 14
                    height: 2
                    radius: 2
                    color: Theme.palette.primary
                }

                Behavior on color {
                    ColorAnimation {
                        duration: 200
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: dispatchProcess.exec(["hyprctl", "dispatch", "workspace", String(wsText.wsId)])
                    onWheel: wheel => {
                        const ids = wsRoot.workspaceIds;
                        const currentIndex = ids.indexOf(wsRoot.activeWsId);
                        if (currentIndex < 0)
                            return;

                        if (wheel.angleDelta.y > 0) {
                            dispatchProcess.exec(["hyprctl", "dispatch", "workspace", String(ids[Math.max(0, currentIndex - 1)])]);
                        } else {
                            dispatchProcess.exec(["hyprctl", "dispatch", "workspace", String(ids[Math.min(ids.length - 1, currentIndex + 1)])]);
                        }
                    }
                }
            }
        }
    }
}
