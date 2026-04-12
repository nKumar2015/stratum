import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "../globals/DaemonRpc.js" as DaemonRpc
import "../globals"

Item {
    id: root

    property string monitorName: ""

    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 16
    Layout.preferredHeight: 16

    readonly property int iconFadeMs: 150
    readonly property int hoverOpenDelayMs: 350
    readonly property int hoverExitGraceMs: 420
    readonly property bool preferDaemonBootstrap: true

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

    function bootstrapStatus() {
        if (preferDaemonBootstrap && DaemonRpc.canUse())
            daemonBluetoothProc.running = true;
        else
            bluetoothProc.running = true;
    }

    function updateStatus(output) {
        // Skip polling sync while settings are open or active scan is running.
        if (GlobalState.showBluetoothSettings || GlobalState.bluetoothScanning)
            return;

        const payload = parseCliJson(output);
        if (!payload)
            return;

        let source = null;
        if (payload.jsonrpc === "2.0" && payload.result && payload.result.ok === true && payload.result.bluetooth)
            source = payload.result.bluetooth;
        else if (payload.ok === true)
            source = payload;

        if (!source)
            return;

        const raw = String(source.state || "").trim().toLowerCase();
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
        id: daemonBluetoothProc
        command: DaemonRpc.command("bluetooth.status", {})
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = root.parseCliJson(result);
                if (payload && payload.jsonrpc === "2.0" && payload.result && payload.result.ok === true) {
                    DaemonRpc.recordSuccess();
                    GlobalState.daemonAvailable = true;
                    root.updateStatus(result);
                } else {
                    DaemonRpc.recordFailure();
                    GlobalState.daemonAvailable = false;
                    bluetoothProc.running = true;
                    return;
                }
            }
        }
    }

    Process {
        id: bluetoothProc
        command: ["stratum-cli", "bluetooth", "check"]
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result)
                    root.updateStatus(result);
            }
        }
    }

    Component.onCompleted: root.bootstrapStatus()

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: bluetoothHover.containsMouse ? Theme.palette.secondary : Theme.palette.textMain

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
