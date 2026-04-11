pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io

import "../globals"
import "../components"

PanelWindow {
    id: hoverMenu

    screen: {
        const targetName = GlobalState.popupMonitorName || Hyprland.focusedMonitor?.name || "";
        const screens = Quickshell.screens || [];
        for (let index = 0; index < screens.length; index++) {
            const candidate = screens[index];
            const monitor = Hyprland.monitorFor(candidate);
            if (monitor?.name === targetName)
                return candidate;
        }
        return null;
    }

    anchors.left: true
    anchors.top: true

    margins.left: 44
    margins.top: {
        const iconY = GlobalState.batteryIconY;
        const minTop = 8;
        const bottomInset = 8;
        const desiredTop = iconY <= 0 ? 100 : Math.round(iconY - implicitHeight / 2) - 40;
        const screenHeight = hoverMenu.screen ? hoverMenu.screen.height : 0;
        if (screenHeight <= 0)
            return Math.max(minTop, desiredTop);
        const maxTop = Math.max(minTop, screenHeight - implicitHeight - bottomInset);
        return Math.max(minTop, Math.min(desiredTop, maxTop));
    }

    exclusiveZone: -1

    implicitWidth: 280
    implicitHeight: Math.max(col.implicitHeight + 24, 180)

    visible: GlobalState.showBatteryHoverMenu
    color: "transparent"

    property bool loading: false
    property bool switching: false
    property string statusMsg: ""

    property int batteryPct: 0
    property string batteryState: "unknown"
    property string projectedLife: "Unknown"
    property string screenOnTime: "Unknown"
    property string chargingInfo: ""
    property string activeProfile: "balanced"

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

    function stateText() {
        if (batteryState === "charging")
            return "Charging";
        if (batteryState === "discharging")
            return "On Battery";
        if (batteryState === "fully-charged")
            return "Plugged In";
        if (batteryState === "pending-charge")
            return "Plugged In";
        return "Battery";
    }

    function profileLabel(name) {
        if (name === "low-power")
            return "Saver";
        if (name === "balanced-performance")
            return "Performance";
        return "Normal";
    }

    function projectedLabel() {
        if (batteryState === "charging")
            return "To Full";
        if (batteryState === "discharging")
            return "Remaining";
        if (batteryState === "fully-charged" || batteryState === "pending-charge")
            return "Status";
        return "Projected";
    }

    function projectedValue() {
        if (batteryState === "fully-charged")
            return "Fully Charged";
        if (batteryState === "pending-charge")
            return "Not Charging";
        return projectedLife;
    }

    function projectedColor() {
        if (batteryState === "fully-charged")
            return Theme.palette.success;
        if (batteryState === "pending-charge")
            return Theme.palette.warning;
        return Theme.palette.success;
    }

    function loadStatus() {
        loading = true;
        statusProc.running = true;
    }

    function setProfile(profileName) {
        if (switching || !profileName || activeProfile === profileName)
            return;

        switching = true;
        statusMsg = "Switching to " + profileLabel(profileName) + "...";
        actionProc.command = ["stratum-cli", "battery", "set-profile", profileName];
        actionProc.running = true;
    }

    Process {
        id: statusProc
        command: ["stratum-cli", "battery", "status"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;

                const raw = this.text.trim();
                const payload = hoverMenu.parseCliJson(raw);
                if (!payload) {
                    hoverMenu.statusMsg = "Battery info unavailable";
                    statusClearTimer.restart();
                    return;
                }

                if (payload.ok !== true) {
                    hoverMenu.statusMsg = String(payload.error || "Battery info unavailable");
                    statusClearTimer.restart();
                    return;
                }

                const battery = (payload.battery && typeof payload.battery === "object") ? payload.battery : {};
                const parsedPct = parseInt(String(battery.pct || "0"));
                hoverMenu.batteryPct = isNaN(parsedPct) ? 0 : Math.max(0, Math.min(100, parsedPct));
                hoverMenu.batteryState = String(battery.state || "unknown").trim();
                hoverMenu.projectedLife = String(battery.projected_text || "Unknown").trim();
                hoverMenu.screenOnTime = String(battery.screen_on_time || "Unknown").trim();

                hoverMenu.chargingInfo = String(payload.charging_info || "").trim();
                hoverMenu.activeProfile = String(payload.profile || "balanced").trim();
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.switching = false;

                const payload = hoverMenu.parseCliJson(result);
                if (!payload || payload.ok !== true) {
                    hoverMenu.statusMsg = payload && payload.error ? String(payload.error) : "Profile switch failed";
                    statusClearTimer.restart();
                } else {
                    hoverMenu.statusMsg = "Power mode updated";
                    statusClearTimer.restart();
                }

                hoverMenu.loadStatus();
            }
        }
    }

    Timer {
        id: statusClearTimer
        interval: 2200
        repeat: false
        onTriggered: hoverMenu.statusMsg = ""
    }

    onVisibleChanged: {
        if (visible) {
            statusMsg = "";
            loadStatus();
        } else {
            hideTimer.stop();
        }
    }

    Timer {
        id: hideTimer
        interval: 350
        repeat: false
        running: false
        onTriggered: {
            if (!GlobalState.batteryHoverIntent && !menuHoverHandler.hovered)
                GlobalState.showBatteryHoverMenu = false;
        }
    }

    Connections {
        target: GlobalState
        function onBatteryHoverIntentChanged() {
            if (GlobalState.batteryHoverIntent || menuHoverHandler.hovered)
                hideTimer.stop();
            else
                hideTimer.restart();
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.palette.bgMain
        border.color: Theme.palette.borderActive
        border.width: 1
        radius: 10

        HoverHandler {
            id: menuHoverHandler
            onHoveredChanged: GlobalState.batteryHoverIntent = hovered
        }

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "󰂄  Battery"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 13
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "󰅖"
                    color: Theme.palette.error
                    font.pixelSize: 13
                    font.family: Theme.palette.font

                    MouseArea {
                        id: closeHover
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: GlobalState.showBatteryHoverMenu = false
                    }

                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }
                }
            }

            Divider {}

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: hoverMenu.stateText() + " · " + hoverMenu.batteryPct + "%"
                    color: Theme.palette.textMain
                    font.pixelSize: 12
                    font.family: Theme.palette.font
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: hoverMenu.loading ? "Loading..." : ""
                    visible: hoverMenu.loading
                    color: Theme.palette.textMain
                    opacity: 0.5
                    font.pixelSize: 11
                    font.family: Theme.palette.font
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Text {
                    text: "Screen On"
                    color: Theme.palette.textMain
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.palette.font
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: hoverMenu.screenOnTime
                    color: Theme.palette.textMain
                    font.pixelSize: 11
                    font.family: Theme.palette.font
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Text {
                    text: hoverMenu.projectedLabel()
                    color: Theme.palette.textMain
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.palette.font
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: hoverMenu.projectedValue()
                    color: hoverMenu.projectedColor()
                    font.pixelSize: 11
                    font.family: Theme.palette.font
                    font.bold: hoverMenu.batteryState === "fully-charged"
                }
            }

            RowLayout {
                id: chargingRow
                visible: (hoverMenu.batteryState === "charging" || hoverMenu.batteryState === "pending-charge") && hoverMenu.chargingInfo.length > 0
                Layout.fillWidth: true
                spacing: 6

                SequentialAnimation on opacity {
                    running: chargingRow.visible
                    loops: Animation.Infinite
                    NumberAnimation {
                        to: 0.4
                        duration: 900
                        easing.type: Easing.InOutSine
                    }
                    NumberAnimation {
                        to: 1.0
                        duration: 900
                        easing.type: Easing.InOutSine
                    }
                }

                Text {
                    text: "󱐋  Rate"
                    color: Theme.palette.primary
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.palette.font
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: hoverMenu.chargingInfo
                    color: Theme.palette.primary
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.palette.font
                }
            }

            Divider {}

            Text {
                text: "Power Mode"
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.bold: true
                font.family: Theme.palette.font
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Repeater {
                    model: [
                        {
                            key: "low-power",
                            label: "Saver"
                        },
                        {
                            key: "balanced",
                            label: "Normal"
                        },
                        {
                            key: "balanced-performance",
                            label: "Performance"
                        }
                    ]

                    delegate: Rectangle {
                        id: powerOption
                        required property var modelData
                        property bool selected: hoverMenu.activeProfile === modelData.key
                        property bool disabled: hoverMenu.switching

                        Layout.fillWidth: true
                        height: 30
                        radius: 6
                        color: selected ? Theme.palette.bgWidget : (modeHover.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget)
                        border.width: 1
                        border.color: selected ? Theme.palette.borderActive : Theme.palette.borderInactive
                        Text {
                            anchors.centerIn: parent
                            text: powerOption.modelData.label
                            color: Theme.palette.textMain
                            font.pixelSize: 10
                            font.family: Theme.palette.font
                            font.bold: true
                        }

                        MouseArea {
                            id: modeHover
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: powerOption.disabled ? Qt.ArrowCursor : Qt.PointingHandCursor
                            enabled: !powerOption.disabled
                            onClicked: hoverMenu.setProfile(powerOption.modelData.key)
                        }
                    }
                }
            }

            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.palette.primary
                font.pixelSize: 11
                font.family: Theme.palette.font
                Layout.fillWidth: true
                elide: Text.ElideRight
            }
        }
    }
}
