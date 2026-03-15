import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io

import "../theme"
import "../globals"

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
        if (iconY <= 0)
            return 100;
        return Math.max(8, Math.round(iconY - implicitHeight / 2) - 40);
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
            return Theme.green;
        if (batteryState === "pending-charge")
            return Theme.yellow;
        return Theme.text;
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
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/battery_menu.sh", "set-profile", profileName];
        actionProc.running = true;
    }

    Process {
        id: statusProc
        command: ["sh", Quickshell.shellDir + "/scripts/battery_menu.sh", "hover-status"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;

                const raw = this.text.trim();
                if (!raw || raw.startsWith("__ERROR__")) {
                    hoverMenu.statusMsg = raw.startsWith("__ERROR__") ? raw.replace("__ERROR__|", "") : "Battery info unavailable";
                    statusClearTimer.restart();
                    return;
                }

                const lines = raw.split("\n");
                for (let i = 0; i < lines.length; i++) {
                    const parts = lines[i].split("|");
                    const type = (parts[0] || "").trim();

                    if (type === "BATTERY") {
                        const parsedPct = parseInt(parts[1]);
                        hoverMenu.batteryPct = isNaN(parsedPct) ? 0 : Math.max(0, Math.min(100, parsedPct));
                        hoverMenu.batteryState = (parts[2] || "unknown").trim();
                        hoverMenu.projectedLife = (parts[3] || "Unknown").trim();
                        hoverMenu.screenOnTime = (parts[4] || "Unknown").trim();
                    } else if (type === "CHARGING") {
                        hoverMenu.chargingInfo = (parts[1] || "").trim();
                    } else if (type === "PROFILE") {
                        hoverMenu.activeProfile = (parts[1] || "balanced").trim();
                    }
                }
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.switching = false;

                if (result.startsWith("__ERROR__") || result.toLowerCase().indexOf("failed") !== -1) {
                    hoverMenu.statusMsg = "Profile switch failed";
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
        color: Theme.background
        border.color: Theme.grey
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
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 13
                    font.bold: true
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: "󰅖"
                    color: closeHover.containsMouse ? Theme.red : Theme.text
                    font.pixelSize: 13
                    font.family: Theme.font

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

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.grey
            }

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: hoverMenu.stateText() + " · " + hoverMenu.batteryPct + "%"
                    color: Theme.text
                    font.pixelSize: 12
                    font.family: Theme.font
                    font.bold: true
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: hoverMenu.loading ? "Loading..." : ""
                    visible: hoverMenu.loading
                    color: Theme.text
                    opacity: 0.5
                    font.pixelSize: 11
                    font.family: Theme.font
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Text {
                    text: "Screen On"
                    color: Theme.text
                    opacity: 0.6
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.font
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: hoverMenu.screenOnTime
                    color: Theme.text
                    font.pixelSize: 11
                    font.family: Theme.font
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Text {
                    text: hoverMenu.projectedLabel()
                    color: Theme.text
                    opacity: 0.6
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.font
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: hoverMenu.projectedValue()
                    color: hoverMenu.projectedColor()
                    font.pixelSize: 11
                    font.family: Theme.font
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
                    NumberAnimation { to: 0.4; duration: 900; easing.type: Easing.InOutSine }
                    NumberAnimation { to: 1.0; duration: 900; easing.type: Easing.InOutSine }
                }

                Text {
                    text: "󱐋  Rate"
                    color: Theme.green
                    opacity: 0.85
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.font
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: hoverMenu.chargingInfo
                    color: Theme.green
                    font.pixelSize: 11
                    font.bold: true
                    font.family: Theme.font
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.grey
                opacity: 0.7
            }

            Text {
                text: "Power Mode"
                color: Theme.text
                font.pixelSize: 11
                font.bold: true
                font.family: Theme.font
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 6

                Repeater {
                    model: [
                        { key: "low-power", label: "Saver" },
                        { key: "balanced", label: "Normal" },
                        { key: "balanced-performance", label: "Performance" }
                    ]

                    delegate: Rectangle {
                        required property var modelData
                        property bool selected: hoverMenu.activeProfile === modelData.key
                        property bool disabled: hoverMenu.switching

                        Layout.fillWidth: true
                        height: 30
                        radius: 6
                        color: selected ? Theme.activeWs : (modeHover.containsMouse ? Theme.grey : Theme.black)
                        border.width: 1
                        border.color: selected ? Theme.activeWs : Theme.grey

                        Text {
                            anchors.centerIn: parent
                            text: modelData.label
                            color: selected ? Theme.black : Theme.text
                            font.pixelSize: 10
                            font.family: Theme.font
                            font.bold: true
                        }

                        MouseArea {
                            id: modeHover
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: disabled ? Qt.ArrowCursor : Qt.PointingHandCursor
                            enabled: !disabled
                            onClicked: hoverMenu.setProfile(modelData.key)
                        }
                    }
                }
            }

            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.blue
                font.pixelSize: 11
                font.family: Theme.font
                opacity: 0.9
                Layout.fillWidth: true
                elide: Text.ElideRight
            }
        }
    }
}
