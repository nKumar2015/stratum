//@ pragma UseQApplication
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io
import QtQuick
import QtQuick.Layouts

import "sidebar"
import "modules"
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
    AudioMenu {}
    BatteryHoverMenu {}
    DashboardMenu {
        id: dashboardMenu
    }
    ThemeSwitcher {}
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

    IpcHandler {
        target: "daemon"

        function audio(payloadText: string): void {
            GlobalState.applyDaemonAudioSnapshot(payloadText);
        }

        function wifi(payloadText: string): void {
            GlobalState.applyDaemonWifiSnapshot(payloadText);
        }

        function bluetooth(payloadText: string): void {
            GlobalState.applyDaemonBluetoothSnapshot(payloadText);
        }

        function music(payloadText: string): void {
            MusicProvider.applyDaemonMusicSnapshot(payloadText);
        }

        function dashboard(payloadText: string): void {
            dashboardMenu.applyDaemonDashboardSnapshot(payloadText);
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
                width: panelWindow.sidebarWidth
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.bottom: parent.bottom

                color: Theme.palette.bgMain
                clip: true
                border.width: 1
                border.color: "transparent"

                ColumnLayout {
                    anchors.fill: parent
                    anchors.topMargin: panelWindow.sidebarPadding
                    anchors.bottomMargin: panelWindow.sidebarPadding
                    anchors.leftMargin: 0
                    anchors.rightMargin: 0
                    spacing: panelWindow.sidebarSpacing

                    Text {
                        text: ""
                        color: Theme.palette.primary
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
                        Layout.preferredWidth: panelWindow.quickPanelWidth
                        Layout.preferredHeight: panelWindow.quickPanelHeight
                        radius: 15
                        color: Theme.palette.bgWidget
                        border.width: 1
                        border.color: Theme.palette.borderActive

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.topMargin: panelWindow.quickPanelInnerMargin
                            anchors.bottomMargin: panelWindow.quickPanelInnerMargin
                            spacing: panelWindow.quickPanelSpacing

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
                        Layout.topMargin: panelWindow.batteryVerticalNudge
                        Layout.bottomMargin: panelWindow.batteryVerticalNudge
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
    }
}
