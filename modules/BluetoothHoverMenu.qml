import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: hoverMenu

    // Anchor to the top-left corner so margins position it precisely
    anchors.left: true
    anchors.top: true

    // Place menu flush right of the 40px visible sidebar (+ 4px gap)
    margins.left: 44
    // Vertically center on the Bluetooth icon using its captured screen Y
    margins.top: {
        const iconY = GlobalState.bluetoothIconY;
        if (iconY <= 0)
            return 100;
        return Math.max(8, Math.round(iconY - implicitHeight / 2));
    }

    // Don't push other windows aside
    exclusiveZone: -1

    implicitWidth: 260
    implicitHeight: Math.max(col.implicitHeight + 24, 96)

    visible: GlobalState.showBluetoothHoverMenu
    color: "transparent"

    property bool loading: false
    property var devices: []
    property string statusMsg: ""
    property string pendingMac: ""

    function loadDevices() {
        loading = true;
        devProc.running = true;
    }

    function connectDevice(mac, name) {
        pendingMac = mac;
        statusMsg = "Connecting to " + name + "...";
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "hover-connect", mac];
        actionProc.running = true;
    }

    function disconnectDevice(mac, name) {
        pendingMac = mac;
        statusMsg = "Disconnecting " + name + "...";
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "hover-disconnect", mac];
        actionProc.running = true;
    }

    Process {
        id: devProc
        command: ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "hover-list"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;
                const text = this.text.trim();
                if (!text || text.startsWith("__ERROR__")) {
                    hoverMenu.devices = [];
                    if (text.startsWith("__ERROR__")) {
                        hoverMenu.statusMsg = "bluetoothctl not found";
                        statusClearTimer.restart();
                    }
                    return;
                }
                const lines = text.split("\n");
                const parsed = [];
                for (let i = 0; i < lines.length; i++) {
                    const parts = lines[i].split("|");
                    if (parts.length < 2)
                        continue;
                    const mac = parts[0].trim();
                    if (!mac)
                        continue;
                    parsed.push({
                        mac: mac,
                        name: parts[1].trim() || mac,
                        connected: (parts[2] || "no").trim()
                    });
                }
                parsed.sort((a, b) => {
                    if (a.connected === "yes" && b.connected !== "yes")
                        return -1;
                    if (b.connected === "yes" && a.connected !== "yes")
                        return 1;
                    return a.name.localeCompare(b.name);
                });
                hoverMenu.devices = parsed;
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.pendingMac = "";
                if (result.startsWith("__ERROR__") || result.toLowerCase().indexOf("failed") !== -1) {
                    hoverMenu.statusMsg = "Action failed";
                    statusClearTimer.restart();
                } else {
                    hoverMenu.statusMsg = "";
                }
                hoverMenu.loadDevices();
            }
        }
    }

    Timer {
        id: statusClearTimer
        interval: 3000
        running: false
        repeat: false
        onTriggered: hoverMenu.statusMsg = ""
    }

    onVisibleChanged: {
        if (visible) {
            devices = [];
            statusMsg = "";
            pendingMac = "";
            loadDevices();
        } else {
            hideTimer.stop();
        }
    }

    Timer {
        id: hideTimer
        interval: 800
        repeat: false
        running: false
        onTriggered: {
            if (!GlobalState.bluetoothHoverIntent)
                GlobalState.showBluetoothHoverMenu = false;
        }
    }

    Connections {
        target: GlobalState
        function onBluetoothHoverIntentChanged() {
            if (GlobalState.bluetoothHoverIntent)
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

        // Track hover over the whole panel so we can auto-hide on exit.
        // HoverHandler is used instead of MouseArea so child MouseAreas don't
        // steal events and cause premature exit detection.
        HoverHandler {
            onHoveredChanged: GlobalState.bluetoothHoverIntent = hovered
        }

        ColumnLayout {
            id: col
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.margins: 12
            spacing: 8

            // ── Header ──────────────────────────────────────────────────
            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "󰂯  Quick Connect"
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 13
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "󰅖"
                    color: closeHover.containsMouse ? Theme.red : Theme.text
                    font.pixelSize: 13
                    font.family: Theme.font

                    MouseArea {
                        id: closeHover
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: GlobalState.showBluetoothHoverMenu = false
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

            // ── Status line (shown only when non-empty) ─────────────────
            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.blue
                font.pixelSize: 11
                font.family: Theme.font
                Layout.fillWidth: true
                elide: Text.ElideRight
            }

            // ── Loading placeholder ──────────────────────────────────────
            Text {
                visible: hoverMenu.loading && hoverMenu.devices.length === 0
                text: "Loading..."
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            // ── Empty state ──────────────────────────────────────────────
            Text {
                visible: !hoverMenu.loading && hoverMenu.devices.length === 0 && hoverMenu.statusMsg.length === 0
                text: "No paired devices"
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            // ── Device rows (max 6) ──────────────────────────────────────
            Repeater {
                model: hoverMenu.devices.slice(0, 6)

                delegate: Rectangle {
                    required property var modelData

                    Layout.fillWidth: true
                    height: 36
                    radius: 6
                    color: rowHover.containsMouse ? Theme.grey : "transparent"

                    property bool isConnected: modelData.connected === "yes"
                    property bool isPending: hoverMenu.pendingMac === modelData.mac

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: isConnected ? "󰂱" : "󰂯"
                            color: isConnected ? Theme.blue : Theme.text
                            font.pixelSize: 15
                            font.family: Theme.font
                        }

                        Text {
                            text: modelData.name
                            color: Theme.text
                            font.pixelSize: 12
                            font.family: Theme.font
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Text {
                            text: isPending ? "󰔟" : (isConnected ? "●" : "○")
                            color: {
                                if (isPending)
                                    return Theme.yellow;
                                if (isConnected)
                                    return Theme.green;
                                return "#5c6370";
                            }
                            font.pixelSize: isPending ? 14 : 10
                            font.family: Theme.font
                        }
                    }

                    MouseArea {
                        id: rowHover
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !isPending && hoverMenu.pendingMac === ""
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            if (isConnected)
                                hoverMenu.disconnectDevice(modelData.mac, modelData.name);
                            else
                                hoverMenu.connectDevice(modelData.mac, modelData.name);
                        }
                    }
                }
            }

            // ── Overflow indicator ───────────────────────────────────────
            Text {
                visible: hoverMenu.devices.length > 6
                text: "+" + (hoverMenu.devices.length - 6) + " more in full settings"
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 11
                font.family: Theme.font
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.grey
            }

            // ── Footer: open full settings ───────────────────────────────
            Text {
                text: "Open full settings →"
                color: fullMenuHover.containsMouse ? Theme.blue : Theme.text
                font.pixelSize: 11
                font.family: Theme.font
                opacity: fullMenuHover.containsMouse ? 1.0 : 0.6
                Layout.bottomMargin: 0

                MouseArea {
                    id: fullMenuHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        GlobalState.showBluetoothHoverMenu = false;
                        GlobalState.showBluetoothSettings = true;
                    }
                }

                Behavior on color {
                    ColorAnimation {
                        duration: 100
                    }
                }
            }
        }
    }
}
