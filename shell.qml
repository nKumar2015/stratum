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
    // Phase 2: Centralized IPC
    IpcModule {}

    // Phase 1: Background Logic (Always Active)
    LockScreen {}
    SystemOsd {}
    NotificationListener {}

    function updateLoader(loader, state, source, monitorName) {
        if (state) {
            console.log("[Shell] Loading " + source);
            // Don't pass the screen property to standard XDG windows!
            loader.setSource("modules/" + source);
        } else {
            console.log("[Shell] Destroying " + source);
            if (loader.item && typeof loader.item.cleanupWindow === "function") {
                loader.item.cleanupWindow();
            }
            loader.source = ""; // Destroys the window
        }
    }

    Connections {
        target: PowerState
        function onShowPowerMenuChanged() {
            updateLoader(powerLoader, PowerState.showPowerMenu, "PowerMenu.qml", GlobalState.popupMonitorName);
        }
        function onShowBatteryHoverMenuChanged() {
            updateLoader(batteryHoverLoader, PowerState.showBatteryHoverMenu, "BatteryHoverMenu.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: NotificationState
        function onShowCenterChanged() {
            updateLoader(notificationLoader, NotificationState.showCenter, "NotificationCenter.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: BluetoothState
        function onShowMenuChanged() {
            updateLoader(bluetoothLoader, BluetoothState.showMenu, "BluetoothMenu.qml", GlobalState.popupMonitorName);
        }
        function onShowHoverMenuChanged() {
            updateLoader(bluetoothHoverLoader, BluetoothState.showHoverMenu, "BluetoothHoverMenu.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: WifiState
        function onShowMenuChanged() {
            updateLoader(wifiLoader, WifiState.showMenu, "WifiMenu.qml", GlobalState.popupMonitorName);
        }
        function onShowHoverMenuChanged() {
            updateLoader(wifiHoverLoader, WifiState.showHoverMenu, "WifiHoverMenu.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: AudioState
        function onShowMenuChanged() {
            updateLoader(audioLoader, AudioState.showMenu, "AudioMenu.qml", GlobalState.popupMonitorName);
        }
        function onShowHoverMenuChanged() {
            updateLoader(audioHoverLoader, AudioState.showHoverMenu, "AudioHoverMenu.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: DashboardState
        function onShowMenuChanged() {
            updateLoader(dashboardLoader, DashboardState.showMenu, "DashboardMenu.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: GlobalState
        function onShowThemeSwitcherChanged() {
            updateLoader(themeLoader, GlobalState.showThemeSwitcher, "ThemeSwitcher.qml", GlobalState.popupMonitorName);
        }
        function onShowScreenshotViewerChanged() {
            updateLoader(screenshotLoader, GlobalState.showScreenshotViewer, "ScreenshotViewer.qml", GlobalState.popupMonitorName);
        }
    }

    Connections {
        target: PolkitState
        function onActiveChanged() {
            console.log("[Shell] Polkit active status changed: " + PolkitState.active);
            updateLoader(polkitLoader, PolkitState.active, "PolkitDialog.qml", GlobalState.popupMonitorName);
        }
    }

    function screenByName(name) {
        const target = String(name || "").trim();
        const screens = Quickshell.screens || [];
        if (!screens.length)
            return null;
        if (!target)
            return screens[0];
        for (let i = 0; i < screens.length; i++) {
            const mon = Hyprland.monitorFor(screens[i]);
            if (mon?.name === target)
                return screens[i];
        }
        return screens[0];
    }

    // Phase 3: Lazy-Loaded UI Components
    Loader {
        id: powerLoader
        onLoaded: {
            console.log("[Shell] PowerMenu loaded");
            if (typeof item.initializeWindow === "function") {
                item.initializeWindow();
            }
            item.requestActivate();
        }
    }

    Loader {
        id: notificationLoader
        onLoaded: console.log("[Shell] NotificationCenter loaded")
    }

    Loader {
        id: bluetoothLoader
        onLoaded: {
            console.log("[Shell] BluetoothMenu successfully built in RAM.");
            if (typeof item.initializeWindow === "function") {
                item.initializeWindow();
            }
            item.show();
            item.requestActivate();
        }
    }

    Loader {
        id: bluetoothHoverLoader
    }

    Loader {
        id: wifiLoader
        onLoaded: {
            console.log("[Shell] WifiMenu successfully built in RAM.");
            if (typeof item.initializeWindow === "function") {
                item.initializeWindow();
            }
            item.show();
            item.requestActivate();
        }
    }

    Loader {
        id: wifiHoverLoader
    }

    Loader {
        id: audioHoverLoader
    }

    Loader {
        id: audioLoader
        onLoaded: {
            console.log("[Shell] AudioMenu successfully built in RAM.");
            if (typeof item.initializeWindow === "function") {
                item.initializeWindow();
            }
            item.show();
            item.requestActivate();
        }
    }

    Loader {
        id: batteryHoverLoader
    }

    Loader {
        id: dashboardLoader
        onLoaded: console.log("[Shell] DashboardLoader loaded")
    }

    Loader {
        id: themeLoader
        onLoaded: console.log("[Shell] ThemeSwitcher loaded")
    }

    Loader {
        id: screenshotLoader
        onLoaded: {
            console.log("[Shell] ScreenshotViewer loaded");
            if (typeof item.initializeWindow === "function") {
                item.initializeWindow();
            }
            item.show();
            item.requestActivate();
        }
    }

    Loader {
        id: polkitLoader
        onLoaded: console.log("[Shell] PolkitDialog loaded")
    }

    Variants {
        model: Quickshell.screens

        ScreenshotToolbar {
            property var modelData
            screen: modelData
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
