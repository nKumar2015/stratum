import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Widgets
import QtQuick.Layouts

import "../theme"

Item {
    required property var monitor

    Layout.fillHeight: true
    Layout.preferredWidth: 36
    clip: true

    readonly property var monitorWorkspace: monitor?.activeWorkspace || null
    readonly property var monitorWindows: monitorWorkspace?.toplevels?.values || []
    readonly property var activatedMonitorWindow: {
        for (let index = 0; index < monitorWindows.length; index++) {
            const candidate = monitorWindows[index];
            if (candidate?.activated)
                return candidate;
        }
        return null;
    }
    readonly property var displayWindow: {
        const active = Hyprland.activeToplevel;
        if (active && active.workspace?.id === monitorWorkspace?.id)
            return active;

        if (activatedMonitorWindow)
            return activatedMonitorWindow;

        if (monitorWindows.length > 0)
            return monitorWindows[monitorWindows.length - 1];

        return null;
    }
    readonly property string displayAppId: String(displayWindow?.appId || displayWindow?.wayland?.appId || displayWindow?.handle?.appId || "desktop").toLowerCase()

    Item {
        anchors.centerIn: parent

        width: parent.height
        height: parent.width
        rotation: 270

        RowLayout {
            anchors.centerIn: parent
            spacing: 8
            width: Math.min(implicitWidth, parent.width - 20)
            Image {
                id: appIcon
                source: Quickshell.iconPath(displayAppId, "application-x-executable")

                Layout.preferredWidth: 16
                Layout.preferredHeight: 16
                Layout.alignment: Qt.AlignVCenter

                visible: appText.text !== "\uf4a9  Desktop"
            }

            Text {
                id: appText
                text: displayWindow?.title || "\uf4a9  Desktop"
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
