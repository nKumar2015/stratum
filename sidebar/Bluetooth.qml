import QtQuick
import Quickshell
import QtQuick.Layouts
import Quickshell.Io
import "../theme"
import "../globals"

Item {
    id: root

    property string monitorName: ""

    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 16
    Layout.preferredHeight: 16

    readonly property int statusPollMs: 5000
    readonly property int iconFadeMs: 150
    readonly property int hoverOpenDelayMs: 350
    readonly property int hoverExitGraceMs: 420

    // scanning=󰂰, connected=󰂱, powered=󰂯, off=󰂲
    property string icon: GlobalState.bluetoothScanning ? "󰂰" : (GlobalState.bluetoothConnected ? "󰂱" : (GlobalState.bluetoothPowered ? "󰂯" : "󰂲"))

    function parseCliJson(raw) {
        const text = String(raw || "").trim();
        if (!text.length)
            return null;

        try {
            return JSON.parse(text);
        } catch (_error) {
            return null;
        }
    }

    function updateStatus(output) {
        // Skip polling sync while settings are open or active scan is running.
        if (GlobalState.showBluetoothSettings || GlobalState.bluetoothScanning)
            return;

        const payload = parseCliJson(output);
        if (!payload || payload.ok !== true)
            return;

        const raw = String(payload.state || "").trim().toLowerCase();
        if (raw === "connected") {
            GlobalState.bluetoothPowered = true;
            GlobalState.bluetoothConnected = true;
            return;
        }

        if (raw === "on") {
            GlobalState.bluetoothPowered = true;
            GlobalState.bluetoothConnected = false;
            return;
        }

        GlobalState.bluetoothPowered = false;
        GlobalState.bluetoothConnected = false;
    }

    Process {
        id: bluetoothProc
        command: ["stratum-cli", "bluetooth", "check"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result)
                    root.updateStatus(result);
                refreshTimer.start();
            }
        }
    }

    Timer {
        id: refreshTimer
        interval: root.statusPollMs
        repeat: false
        onTriggered: bluetoothProc.running = true
    }

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: bluetoothHover.containsMouse ? Theme.primary : Theme.on_Surface
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: root.iconFadeMs
            }
        }
    }

    Timer {
        id: hoverShowTimer
        interval: root.hoverOpenDelayMs
        repeat: false
        onTriggered: {
            if (bluetoothHover.containsMouse && !GlobalState.showBluetoothSettings)
                GlobalState.showBluetoothHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: root.hoverExitGraceMs
        repeat: false
        onTriggered: {
            if (!bluetoothHover.containsMouse)
                GlobalState.bluetoothHoverIntent = false;
        }
    }

    MouseArea {
        id: bluetoothHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            GlobalState.bluetoothIconY = root.mapToGlobal(0, root.height / 2).y;
            GlobalState.bluetoothHoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            hoverExitGraceTimer.stop();
            GlobalState.bluetoothHoverIntent = false;
            hoverShowTimer.stop();
            GlobalState.showBluetoothHoverMenu = false;
            GlobalState.showBluetoothSettings = !GlobalState.showBluetoothSettings;
        }
    }
}
