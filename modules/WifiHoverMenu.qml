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

    // Flush right of the 40px sidebar + 4px gap
    margins.left: 44
    margins.top: {
        const iconY = GlobalState.wifiIconY;
        const minTop = 8;
        const bottomInset = 8;
        const desiredTop = iconY <= 0 ? 100 : Math.round(iconY - implicitHeight / 2);
        const screenHeight = hoverMenu.screen ? hoverMenu.screen.height : 0;
        if (screenHeight <= 0)
            return Math.max(minTop, desiredTop);
        const maxTop = Math.max(minTop, screenHeight - implicitHeight - bottomInset);
        return Math.max(minTop, Math.min(desiredTop, maxTop));
    }

    exclusiveZone: -1

    implicitWidth: 260
    implicitHeight: Math.max(col.implicitHeight + 24, 80)

    visible: GlobalState.showWifiHoverMenu
    color: "transparent"

    // ── Network state ────────────────────────────────────────────────────
    property bool loading: false
    property var connections: []  // list of { type, ssid, signalPct, ipAddress, gateway }
    property string errorMsg: ""

    function signalBars(pct) {
        const level = pct >= 75 ? 4 : pct >= 50 ? 3 : pct >= 25 ? 2 : pct > 0 ? 1 : 0;
        let s = "";
        for (let i = 0; i < 4; i++)
            s += i < level ? "▮" : "▯";
        return s;
    }

    function loadStatus() {
        loading = true;
        errorMsg = "";
        statusProc.running = true;
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

    // ── Fetch: type + ssid + signal from nmcli ──────────────────────────
    // Output format per active device: TYPE|DEVICE|SSID|SIGNAL|IP|GW
    Process {
        id: statusProc
        command: ["stratum-cli", "wifi", "state", "--hover"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;
                const text = this.text.trim();
                if (!text) {
                    hoverMenu.connections = [];
                    return;
                }

                const payload = hoverMenu.parseCliJson(text);
                if (!payload || payload.ok !== true) {
                    hoverMenu.errorMsg = payload && payload.error ? String(payload.error) : "Network status unavailable";
                    hoverMenu.connections = [];
                    return;
                }

                const entries = Array.isArray(payload.connections) ? payload.connections : [];
                const parsed = [];

                for (let i = 0; i < entries.length; i++) {
                    const entry = entries[i] || {};
                    const type = String(entry.type || "").trim();
                    if (type !== "ethernet" && type !== "wifi")
                        continue;

                    const rawSig = String(entry.signal || "").trim();
                    const parsedSignal = rawSig.length > 0 ? parseInt(rawSig) : -1;
                    parsed.push({
                        type:      type,
                        ssid:      String(entry.connection || ""),
                        signalPct: isNaN(parsedSignal) ? -1 : parsedSignal,
                        ipAddress: String(entry.ip_address || ""),
                        gateway:   String(entry.gateway || "")
                    });
                }

                // Sort: ethernet first, then wifi
                parsed.sort((a, b) => {
                    if (a.type === b.type) return 0;
                    return a.type === "ethernet" ? -1 : 1;
                });
                hoverMenu.connections = parsed;
            }
        }
    }

    onVisibleChanged: {
        if (visible) {
            connections = [];
            errorMsg = "";
            loadStatus();
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
            if (!GlobalState.wifiHoverIntent)
                GlobalState.showWifiHoverMenu = false;
        }
    }

    Connections {
        target: GlobalState
        function onWifiHoverIntentChanged() {
            if (GlobalState.wifiHoverIntent)
                hideTimer.stop();
            else
                hideTimer.restart();
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.background
        border.color: Theme.outlineVariant
        border.width: 1
        radius: 10

        // HoverHandler covers the whole panel without being blocked by child MouseAreas.
        HoverHandler {
            onHoveredChanged: GlobalState.wifiHoverIntent = hovered
        }

        ColumnLayout {
            id: col
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.margins: 12
            spacing: 8

            // ── Header ────────────────────────────────────────────────
            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "  Network"
                    color: Theme.on_Surface
                    font.family: Theme.font
                    font.pixelSize: 13
                    font.bold: true
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: "󰅖"
                    color: closeHover.containsMouse ? Theme.error : Theme.on_Surface
                    font.pixelSize: 13
                    font.family: Theme.font

                    MouseArea {
                        id: closeHover
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: GlobalState.showWifiHoverMenu = false
                    }

                    Behavior on color { ColorAnimation { duration: 100 } }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
            }

            // ── Loading ────────────────────────────────────────────────
            Text {
                visible: hoverMenu.loading
                text: "Loading..."
                color: Theme.on_Surface
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            // ── Error ──────────────────────────────────────────────────
            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length > 0
                text: hoverMenu.errorMsg
                color: Theme.error
                font.pixelSize: 12
                font.family: Theme.font
                Layout.fillWidth: true
                elide: Text.ElideRight
            }

            // ── Disconnected ───────────────────────────────────────────
            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.connections.length === 0
                text: "Not connected"
                color: Theme.on_Surface
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            // ── One block per active connection ────────────────────────
            Repeater {
                model: hoverMenu.connections

                delegate: ColumnLayout {
                    required property var modelData
                    required property int index

                    Layout.fillWidth: true
                    spacing: 6

                    // Divider between connections
                    Rectangle {
                        visible: index > 0
                        Layout.fillWidth: true
                        height: 1
                        color: Theme.outlineVariant
                        opacity: 0.5
                    }

                    // Connection type + name row
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            text: modelData.type === "ethernet" ? "\udb80\ude00" : "󰤨"
                            color: Theme.primary
                            font.pixelSize: 16
                            font.family: Theme.font
                        }

                        Text {
                            text: modelData.ssid.length > 0 ? modelData.ssid : (modelData.type === "ethernet" ? "Ethernet" : "Unknown")
                            color: Theme.on_Surface
                            font.pixelSize: 13
                            font.family: Theme.font
                            font.bold: true
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        // Signal bars (wifi only)
                        Text {
                            visible: modelData.type === "wifi" && modelData.signalPct >= 0
                            text: hoverMenu.signalBars(modelData.signalPct)
                            color: modelData.signalPct >= 50 ? Theme.secondary : Theme.tertiary
                            font.pixelSize: 11
                            font.family: Theme.font
                            font.letterSpacing: -1
                        }
                    }

                    // IP address row
                    RowLayout {
                        visible: modelData.ipAddress.length > 0
                        Layout.fillWidth: true
                        spacing: 6

                        Text {
                            text: "IP"
                            color: Theme.on_Surface
                            opacity: 0.5
                            font.pixelSize: 11
                            font.family: Theme.font
                            font.bold: true
                        }

                        Text {
                            text: modelData.ipAddress
                            color: Theme.on_Surface
                            font.pixelSize: 11
                            font.family: Theme.font
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    // Gateway row
                    RowLayout {
                        visible: modelData.gateway.length > 0
                        Layout.fillWidth: true
                        spacing: 6

                        Text {
                            text: "GW"
                            color: Theme.on_Surface
                            opacity: 0.5
                            font.pixelSize: 11
                            font.family: Theme.font
                            font.bold: true
                        }

                        Text {
                            text: modelData.gateway
                            color: Theme.on_Surface
                            font.pixelSize: 11
                            font.family: Theme.font
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    // Signal percentage row (wifi only)
                    RowLayout {
                        visible: modelData.type === "wifi" && modelData.signalPct >= 0
                        Layout.fillWidth: true
                        spacing: 6

                        Text {
                            text: "Signal"
                            color: Theme.on_Surface
                            opacity: 0.5
                            font.pixelSize: 11
                            font.family: Theme.font
                            font.bold: true
                        }

                        Text {
                            text: modelData.signalPct + "%"
                            color: modelData.signalPct >= 50 ? Theme.secondary : Theme.tertiary
                            font.pixelSize: 11
                            font.family: Theme.font
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
            }

            // ── Footer ─────────────────────────────────────────────────
            Text {
                text: "Open full settings →"
                color: fullMenuHover.containsMouse ? Theme.primary : Theme.on_Surface
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
                        GlobalState.showWifiHoverMenu = false;
                        GlobalState.showWifiSettings = true;
                    }
                }

                Behavior on color { ColorAnimation { duration: 100 } }
            }
        }
    }
}
