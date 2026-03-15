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
    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: panelWindow

            property var modelData
            property var monitor: Hyprland.monitorFor(modelData)
            property string monitorName: monitor?.name || ""

            screen: modelData
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
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 86
                        radius: 15
                        color: Theme.black
                        border.width: 1
                        border.color: Theme.grey

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.topMargin: 6
                            anchors.bottomMargin: 6
                            spacing: 4

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
                        Layout.topMargin: -6
                        Layout.bottomMargin: -6
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
