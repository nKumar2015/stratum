//@ pragma UseQApplication
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts

import "sidebar"
import "theme"

PanelWindow {
    id: rootPanelWindow
    anchors.top: true
    anchors.left: true
    anchors.bottom: true
    implicitWidth: 60
    color: "transparent"

    margins.right: -20

    Rectangle {
        id: sidebar
        width: 40
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.bottom: parent.bottom

        color: Theme.background
        clip: true
        border.width: 1
        border.color: "transparent"

        ColumnLayout {
            anchors.fill: parent

            AppTitle {
                anchors.centerIn: parent
            }

            Workspaces {
                Layout.alignment: Qt.AlignVCenter
            }

            Item {
                Layout.fillHeight: true
            }

            Tray {
                Layout.alignment: Qt.AlignHCenter
                Layout.bottomMargin: 10
            }

            Clock {
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: 20
                Layout.bottomMargin: 20
            }
        }
    }

    // Top Corner
    InvertedCorner {
        anchors.top: sidebar.top
        anchors.left: sidebar.right // Attach to the right edge of the 40px bar
        color: Theme.background
        flip: false
    }

    // Bottom Corner
    InvertedCorner {
        anchors.bottom: sidebar.bottom
        anchors.left: sidebar.right
        flip: true
        color: Theme.background
    }
}
