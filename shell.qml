//@ pragma UseQApplication
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import Quickshell.Io
import QtQuick
import QtQuick.Layouts

import "sidebar"
import "modules"
import "theme"
import "globals"

ShellRoot {
    PowerMenu {}
    LockScreen {}
    SystemOsd {}
    NotificationListener {}
    NotificationCenter {}
    BluetoothMenu {}
    BluetoothHoverMenu {}
    WifiMenu {}
    WifiHoverMenu {}
    AudioHoverMenu {}
    BatteryHoverMenu {}
    DashboardMenu {}
    Variants {
        model: Quickshell.screens

        ScreenshotToolbar {
            property var modelData
            screen: modelData
        }
    }
    ScreenshotViewer {}

    IpcHandler {
        target: "screenshot"

        function start(): void {
            if (!GlobalState.screenshotOverlayOpen)
                GlobalState.screenshotOverlayOpen = true;
        }
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: panelWindow

            property var modelData
            property var monitor: Hyprland.monitorFor(modelData)
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

            screen: modelData
            anchors.top: true
            anchors.left: true
            anchors.bottom: true
            implicitWidth: sidebarWidth + cornerInset
            color: "transparent"

            margins.right: -cornerInset

            Rectangle {
                id: sidebar
                width: sidebarWidth
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.bottom: parent.bottom

                color: Theme.background
                clip: true
                border.width: 1
                border.color: "transparent"

                ColumnLayout {
                    anchors.fill: parent
                    anchors.topMargin: sidebarPadding
                    anchors.bottomMargin: sidebarPadding
                    anchors.leftMargin: 0
                    anchors.rightMargin: 0
                    spacing: sidebarSpacing

                    Text {
                        text: ""
                        color: Theme.secondary
                        font.pixelSize: 20
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Workspaces {
                        monitor: panelWindow.monitor
                        Layout.alignment: Qt.AlignHCenter
                    }

                    AppTitle {
                        monitor: panelWindow.monitor
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Tray {
                        panelWindow: panelWindow
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.preferredWidth: quickPanelWidth
                        Layout.preferredHeight: quickPanelHeight
                        radius: 15
                        color: Theme.surfaceContainerLowest
                        border.width: 1
                        border.color: Theme.outlineVariant

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.topMargin: quickPanelInnerMargin
                            anchors.bottomMargin: quickPanelInnerMargin
                            spacing: quickPanelSpacing

                            Audio {
                                monitorName: panelWindow.monitorName
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Bluetooth {
                                monitorName: panelWindow.monitorName
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Wifi {
                                monitorName: panelWindow.monitorName
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }
                    }

                    Battery {
                        monitorName: panelWindow.monitorName
                        Layout.alignment: Qt.AlignHCenter
                        Layout.topMargin: batteryVerticalNudge
                        Layout.bottomMargin: batteryVerticalNudge
                    }

                    Clock {
                        Layout.alignment: Qt.AlignHCenter
                    }
                }
            }

            InvertedCorner {
                anchors.top: sidebar.top
                anchors.left: sidebar.right
                color: Theme.background
                flip: false
            }

            InvertedCorner {
                anchors.bottom: sidebar.bottom
                anchors.left: sidebar.right
                flip: true
                color: Theme.background
            }
        }
    }
}
