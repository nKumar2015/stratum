import QtQuick
import QtQuick.Layouts
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

    // scanning=󰂰, connected=󰂱, powered=󰂯, off=󰂲
    property string icon: GlobalState.bluetoothScanning ? "󰂰" : (GlobalState.bluetoothConnected ? "󰂱" : (GlobalState.bluetoothPowered ? "󰂯" : "󰂲"))

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
