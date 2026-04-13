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
    property string icon: BluetoothState.scanning ? "󰂰" : (BluetoothState.connected ? "󰂱" : (BluetoothState.powered ? "󰂯" : "󰂲"))

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
            if (bluetoothHover.containsMouse && !BluetoothState.showSettings)
                BluetoothState.showHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: root.hoverExitGraceMs
        repeat: false
        onTriggered: {
            if (!bluetoothHover.containsMouse)
                BluetoothState.hoverIntent = false;
        }
    }

    MouseArea {
        id: bluetoothHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            BluetoothState.iconY = root.mapToGlobal(0, root.height / 2).y;
            BluetoothState.hoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            console.log("[Sidebar][Bluetooth] Icon clicked. Monitor: " + root.monitorName + ", New State: " + !BluetoothState.showMenu);
            hoverExitGraceTimer.stop();
            BluetoothState.hoverIntent = false;
            hoverShowTimer.stop();
            BluetoothState.showHoverMenu = false;
            BluetoothState.showMenu = !BluetoothState.showMenu;
        }
    }
}
