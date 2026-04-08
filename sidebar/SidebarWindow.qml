import Quickshell
import Quickshell.Hyprland
import QtQuick
import QtQuick.Layouts

import "../globals"

PanelWindow {
    id: sidebarWindow

    property var modelData

    screen: modelData
    property var monitor: Hyprland.monitorFor(screen)
    property string monitorName: monitor?.name || ""
    readonly property int cornerInset: 20
    readonly property int sidebarWidth: 40
    readonly property int sidebarPadding: 12
    readonly property int sidebarSpacing: 16
    readonly property int quickPanelWidth: 30
    readonly property int quickPanelHeight: 86
    readonly property int quickPanelInnerMargin: 6
    readonly property int quickPanelSpacing: 4
    readonly property int batteryVerticalNudge: -6

    anchors.top: true
    anchors.left: true
    anchors.bottom: true
    implicitWidth: sidebarWidth + cornerInset
    color: "transparent"

    margins.right: -cornerInset

    Rectangle {
        id: sidebar
        width: sidebarWindow.sidebarWidth
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.bottom: parent.bottom

        color: Theme.palette.bgMain
        clip: true
        border.width: 1
        border.color: "transparent"

        ColumnLayout {
            anchors.fill: parent
            anchors.topMargin: sidebarWindow.sidebarPadding
            anchors.bottomMargin: sidebarWindow.sidebarPadding
            anchors.leftMargin: 0
            anchors.rightMargin: 0
            spacing: sidebarWindow.sidebarSpacing

            Text {
                text: ""
                color: Theme.palette.surfaceContainerHighest
                font.pixelSize: 20
                Layout.alignment: Qt.AlignHCenter
            }

            Workspaces {
                monitor: sidebarWindow.monitor
                Layout.alignment: Qt.AlignHCenter
            }

            AppTitle {
                Layout.alignment: Qt.AlignHCenter
            }

            Tray {
                panelWindow: sidebarWindow
                Layout.alignment: Qt.AlignHCenter
            }

            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: sidebarWindow.quickPanelWidth
                Layout.preferredHeight: sidebarWindow.quickPanelHeight
                radius: 15
                color: Theme.surfaceContainerLowest
                border.width: 1
                border.color: Theme.outlineVariant

                ColumnLayout {
                    anchors.fill: parent
                    anchors.topMargin: sidebarWindow.quickPanelInnerMargin
                    anchors.bottomMargin: sidebarWindow.quickPanelInnerMargin
                    spacing: sidebarWindow.quickPanelSpacing

                    Audio {
                        monitorName: sidebarWindow.monitorName
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Bluetooth {
                        monitorName: sidebarWindow.monitorName
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Wifi {
                        monitorName: sidebarWindow.monitorName
                        Layout.alignment: Qt.AlignHCenter
                    }
                }
            }

            Battery {
                monitorName: sidebarWindow.monitorName
                Layout.alignment: Qt.AlignHCenter
                Layout.topMargin: sidebarWindow.batteryVerticalNudge
                Layout.bottomMargin: sidebarWindow.batteryVerticalNudge
            }

            Clock {
                Layout.alignment: Qt.AlignHCenter
            }
        }
    }

    InvertedCorner {
        anchors.top: sidebar.top
        anchors.left: sidebar.right
        flip: false
    }

    InvertedCorner {
        anchors.bottom: sidebar.bottom
        anchors.left: sidebar.right
        flip: true
    }
}
