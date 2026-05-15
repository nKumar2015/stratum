pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io

import "../globals/DaemonRpc.js" as DaemonRpc

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

    // Anchor to the top-left corner so margins position it precisely
    anchors.left: true
    anchors.top: true

    // Place menu flush right of the 40px visible sidebar (+ 4px gap)
    margins.left: 44
    // Vertically center on the Bluetooth icon using its captured screen Y
    margins.top: {
        const iconY = BluetoothState.iconY;
        const minTop = 8;
        const bottomInset = 8;
        const desiredTop = iconY <= 0 ? 100 : Math.round(iconY - implicitHeight / 2);
        const screenHeight = hoverMenu.screen ? hoverMenu.screen.height : 0;
        if (screenHeight <= 0)
            return Math.max(minTop, desiredTop);
        const maxTop = Math.max(minTop, screenHeight - implicitHeight - bottomInset);
        return Math.max(minTop, Math.min(desiredTop, maxTop));
    }

    // Don't push other windows aside
    exclusiveZone: -1

    implicitWidth: 260
    implicitHeight: Math.max(col.implicitHeight + 24, 96)

    visible: BluetoothState.showHoverMenu
    color: "transparent"

    property bool loading: false
    property var devices: []
    property string statusMsg: ""
    property string pendingMac: ""

    function loadDevices() {
        loading = true;
        if (DaemonRpc.canUse())
            devProc.command = DaemonRpc.command("bluetooth.list", {});
        else
            devProc.command = ["stratum-cli", "bluetooth", "list", "--hover"];
        devProc.running = true;
    }

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

    function connectDevice(mac, name) {
        pendingMac = mac;
        statusMsg = "Connecting to " + name + "...";
        if (DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.connect", { mac: mac }, 15);
        else
            actionProc.command = ["stratum-cli", "bluetooth", "connect", mac, "--hover"];
        actionProc.running = true;
    }

    function disconnectDevice(mac, name) {
        pendingMac = mac;
        statusMsg = "Disconnecting " + name + "...";
        if (DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.disconnect", { mac: mac }, 15);
        else
            actionProc.command = ["stratum-cli", "bluetooth", "disconnect", mac, "--hover"];
        actionProc.running = true;
    }

    Process {
        id: devProc
        command: ["stratum-cli", "bluetooth", "list", "--hover"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;
                const text = this.text.trim();
                if (!text) {
                    hoverMenu.devices = [];
                    return;
                }

                const payload = hoverMenu.parseCliJson(text);
                
                // Track daemon success/failure if we used RPC
                if (payload && payload.jsonrpc === "2.0") {
                    if (payload.result && payload.result.ok === true) {
                        DaemonRpc.recordSuccess();
                    } else if (!payload.result || payload.result.ok !== true) {
                        DaemonRpc.recordFailure();
                    }
                }
                
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;

                if (!source || source.ok !== true) {
                    hoverMenu.devices = [];
                    hoverMenu.statusMsg = source && source.error ? String(source.error) : "bluetoothctl not found";
                    statusClearTimer.restart();
                    return;
                }

                const rows = Array.isArray(source.devices) ? source.devices : [];
                const parsed = [];

                for (let i = 0; i < rows.length; i++) {
                    const row = rows[i] || {};
                    const mac = String(row.mac || "").trim();
                    if (!mac)
                        continue;
                    parsed.push({
                        mac: mac,
                        name: String(row.name || "").trim() || mac,
                        connected: String(row.connected || "no").trim()
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
                let payload = hoverMenu.parseCliJson(result);
                
                if (payload && payload.jsonrpc === "2.0") {
                    if (payload.result && payload.result.ok === true) {
                        DaemonRpc.recordSuccess();
                    } else {
                        DaemonRpc.recordFailure();
                    }
                    payload = payload.result;
                }
                
                const message = payload ? String(payload.output || payload.error || "") : result;
                if (!payload || payload.ok !== true || message.toLowerCase().indexOf("failed") !== -1 || message.toLowerCase().indexOf("error") !== -1) {
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
            if (!BluetoothState.hoverIntent && !menuHover.hovered)
                BluetoothState.showHoverMenu = false;
        }
    }

    Connections {
        target: BluetoothState
        function onHoverIntentChanged() {
            if (BluetoothState.hoverIntent)
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

        // Track hover over the whole panel so we can auto-hide on exit.
        // HoverHandler is used instead of MouseArea so child MouseAreas don't
        // steal events and cause premature exit detection.
        HoverHandler {
            id: menuHover
            onHoveredChanged: {
                BluetoothState.hoverIntent = hovered;
                if (!hovered)
                    hideTimer.restart();
            }
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
                        onClicked: BluetoothState.showHoverMenu = false
                    }

                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }
                }
            }

            Divider {}

            // ── Status line (shown only when non-empty) ─────────────────
            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.family: Theme.palette.font
                Layout.fillWidth: true
                elide: Text.ElideRight
            }

            // ── Loading placeholder ──────────────────────────────────────
            Text {
                visible: hoverMenu.loading && hoverMenu.devices.length === 0
                text: "Loading..."
                color: Theme.palette.textMain
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.palette.font
            }

            // ── Empty state ──────────────────────────────────────────────
            Text {
                visible: !hoverMenu.loading && hoverMenu.devices.length === 0 && hoverMenu.statusMsg.length === 0
                text: "No paired devices"
                color: Theme.palette.textMain
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.palette.font
            }

            // ── Device rows (max 6) ──────────────────────────────────────
            Repeater {
                model: hoverMenu.devices.slice(0, 6)

                delegate: HoverListRow {
                    id: deviceRow
                    required property var modelData

                    Layout.fillWidth: true
                    rowHeight: 36
                    rowRadius: 6
                    showLabel: false
                    backgroundColor: "transparent"
                    activeBackgroundColor: Theme.palette.bgHover
                    borderColor: "transparent"
                    activeBorderColor: "transparent"
                    contentLeftMargin: 8
                    contentRightMargin: 8

                    property bool isConnected: modelData.connected === "yes"
                    property bool isPending: hoverMenu.pendingMac === modelData.mac
                    isInteractive: !isPending && hoverMenu.pendingMac === ""

                    StatusRow {
                        anchors.fill: parent
                        spacing: 8
                        leadingIconText: deviceRow.isConnected ? "󰂱" : "󰂯"
                        iconColor: deviceRow.isConnected ? Theme.palette.textMain : Theme.palette.textMuted
                        iconPixelSize: 15
                        labelText: deviceRow.modelData.name
                        labelColor: Theme.palette.textMain
                        labelPixelSize: 12
                        valueText: deviceRow.isPending ? "󰔟" : (deviceRow.isConnected ? "●" : "○")
                        valueColor: {
                            if (deviceRow.isPending)
                                return Theme.palette.warning;
                            if (deviceRow.isConnected)
                                return Theme.palette.success;
                            return Theme.palette.textMain;
                        }
                        valuePixelSize: deviceRow.isPending ? 14 : 10
                        valueBold: false
                        valueFillWidth: false
                        valueElideRight: false
                    }

                    onClicked: {
                        if (deviceRow.isConnected)
                            hoverMenu.disconnectDevice(deviceRow.modelData.mac, deviceRow.modelData.name);
                            else
                            hoverMenu.connectDevice(deviceRow.modelData.mac, deviceRow.modelData.name);
                    }
                }
            }

            // ── Overflow indicator ───────────────────────────────────────
            Text {
                visible: hoverMenu.devices.length > 6
                text: "+" + (hoverMenu.devices.length - 6) + " more in full settings"
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.family: Theme.palette.font
            }

            Divider {}

            // ── Footer: open full settings ───────────────────────────────
            Text {
                text: "Open full settings →"
                color: fullMenuHover.containsMouse ? Theme.palette.textMain : Theme.palette.textMuted
                font.pixelSize: 11
                font.family: Theme.palette.font
                Layout.bottomMargin: 0

                MouseArea {
                    id: fullMenuHover
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        BluetoothState.showHoverMenu = false;
                        BluetoothState.showMenu = true;
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
