import Quickshell.Services.UPower
import QtQuick
import QtQuick.Layouts

import "../globals"

ColumnLayout {
    id: root
    property string monitorName: ""
    property bool hoverActive: false
    readonly property int hoverOpenDelayMs: 350
    readonly property int hoverExitGraceMs: 420
    spacing: 0

    // Battery glyphs: unknown/empty=󰂎, charging=󰂄, full=󰁹, then level buckets.
    function getBatteryIcon() {
        if (!UPower.displayDevice.ready)
            return "󰂎";
        let pct = UPower.displayDevice.percentage * 100;
        let state = UPower.displayDevice.state;

        if (state === UPowerDeviceState.Charging)
            return "󰂄";
        if (state === UPowerDeviceState.FullyCharged)
            return "󰁹";
        if (state === UPowerDeviceState.PendingCharge)
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
            return Theme.palette.textMain;
        if (UPower.displayDevice.state === UPowerDeviceState.Charging)
            return Theme.palette.success;
        if (UPower.displayDevice.state === UPowerDeviceState.FullyCharged)
            return Theme.palette.success;
        if (UPower.displayDevice.state === UPowerDeviceState.PendingCharge)
            return Theme.palette.success;
        if (UPower.displayDevice.percentage <= 0.20)
            return Theme.palette.error;
        return Theme.palette.primary;
    }

    // Keep a fixed container so rotating the glyph does not affect layout bounds.
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
            font.family: Theme.palette.font
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
        font.family: Theme.palette.font

        Behavior on color {
            ColorAnimation {
                duration: 250
            }
        }
    }

    Timer {
        id: hoverShowTimer
        interval: root.hoverOpenDelayMs
        repeat: false
        onTriggered: {
            if (root.hoverActive)
                PowerState.showBatteryHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: root.hoverExitGraceMs
        repeat: false
        onTriggered: {
            if (!root.hoverActive)
                PowerState.batteryHoverIntent = false;
        }
    }

    HoverHandler {
        onHoveredChanged: {
            root.hoverActive = hovered;
            if (hovered) {
                hoverExitGraceTimer.stop();
                GlobalState.setPopupMonitorName(root.monitorName);
                PowerState.batteryIconY = root.mapToGlobal(0, root.height / 2).y;
                PowerState.batteryHoverIntent = true;
                hoverShowTimer.start();
            } else {
                hoverShowTimer.stop();
                hoverExitGraceTimer.restart();
            }
        }
    }
}
