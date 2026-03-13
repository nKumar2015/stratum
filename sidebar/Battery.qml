import Quickshell.Services.UPower
import QtQuick
import QtQuick.Layouts

import "../theme"

ColumnLayout {
    id: root
    spacing: 0

    function getBatteryIcon() {
        if (!UPower.displayDevice.ready)
            return "󰂎";
        width: Math.min(implicitWidth, parent.width - 20);
        let pct = UPower.displayDevice.percentage * 100;
        let state = UPower.displayDevice.state;

        if (state === UPowerDeviceState.Charging)
            return "󰂄";
        if (pct >= 90)
            return "󰁹";
        if (pct >= 80)
            return "󰂂";
        if (pct >= 70)
            return "󰂁";
        if (pct >= 60)
            return "󰂀";
        if (pct >= 50)
            return "󰁿";
        if (pct >= 40)
            return "󰁾";
        if (pct >= 30)
            return "󰁽";
        if (pct >= 20)
            return "󰁼";
        if (pct >= 10)
            return "󰁻";
        return "󰂎";
    }

    function getBatteryColor() {
        if (!UPower.displayDevice.ready)
            return Theme.inactiveWs;
        if (UPower.displayDevice.state === UPowerDeviceState.Charging)
            return Theme.green;
        if (UPower.displayDevice.percentage <= 0.20)
            return Theme.red;
        return Theme.activeWs;
    }

    // THE FIX 2: Wrap the rotated text in a fixed-size bounding box
    Item {
        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: 24
        Layout.preferredHeight: 24

        Text {
            anchors.centerIn: parent
            text: root.getBatteryIcon()
            color: root.getBatteryColor()
            font.pixelSize: 24
            rotation: 90
            font.family: Theme.font
            Behavior on color {
                ColorAnimation {
                    duration: 250
                }
            }
        }
    }

    Text {
        Layout.alignment: Qt.AlignHCenter
        text: UPower.displayDevice.ready ? Math.round(UPower.displayDevice.percentage * 100) + "%" : "--"
        color: root.getBatteryColor()
        font.pixelSize: 11
        font.bold: true
        font.family: Theme.font

        Behavior on color {
            ColorAnimation {
                duration: 250
            }
        }
    }
}
