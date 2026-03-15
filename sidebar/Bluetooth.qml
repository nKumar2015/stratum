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

    property string icon: GlobalState.bluetoothScanning ? "󰂰" : (GlobalState.bluetoothConnected ? "󰂱" : (GlobalState.bluetoothPowered ? "󰂯" : "󰂲"))

    function updateStatus(output) {
        if (GlobalState.showBluetoothSettings || GlobalState.bluetoothScanning)
            return;

        const raw = output.trim().toLowerCase();
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
        command: ["sh", Quickshell.shellDir + "/scripts/check_bluetooth.sh"]
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
        interval: 5000
        repeat: false
        onTriggered: bluetoothProc.running = true
    }

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: bluetoothHover.containsMouse ? Theme.blue : Theme.text
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: 150
            }
        }
    }

    Timer {
        id: hoverShowTimer
        interval: 350
        repeat: false
        onTriggered: {
            if (bluetoothHover.containsMouse && !GlobalState.showBluetoothSettings)
                GlobalState.showBluetoothHoverMenu = true;
        }
    }

    MouseArea {
        id: bluetoothHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            GlobalState.setPopupMonitorName(root.monitorName);
            GlobalState.bluetoothIconY = root.mapToGlobal(0, root.height / 2).y;
            GlobalState.bluetoothHoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            GlobalState.bluetoothHoverIntent = false;
            hoverShowTimer.stop();
        }
        onClicked: {
            hoverShowTimer.stop();
            GlobalState.showBluetoothHoverMenu = false;
            GlobalState.showBluetoothSettings = !GlobalState.showBluetoothSettings;
        }
    }
}
