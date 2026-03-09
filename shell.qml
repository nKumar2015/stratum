//@ pragma UseQApplication
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts

import "sidebar"
import "modules"
import "theme"

ShellRoot {
    PowerMenu {}
    LockScreen {}
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
                anchors.topMargin: 12
                anchors.bottomMargin: 12
                anchors.leftMargin: 0
                anchors.rightMargin: 0
                spacing: 16

                Text {
                    text: ""
                    color: Theme.defaultWs
                    font.pixelSize: 20
                    Layout.alignment: Qt.AlignHCenter
                }

                Workspaces {
                    Layout.alignment: Qt.AlignHCenter
                }

                AppTitle {
                    Layout.alignment: Qt.AlignHCenter
                }

                Tray {
                    Layout.alignment: Qt.AlignHCenter
                }

                Battery {
                    Layout.alignment: Qt.AlignHCenter
                }

                Clock {
                    Layout.alignment: Qt.AlignHCenter
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
}
