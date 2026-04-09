import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Quickshell.Io

import "../globals"

Window {
    id: bluetoothMenu

    title: "Bluetooth"
    flags: Qt.Window

    visible: GlobalState.showBluetoothSettings

    width: 700
    height: 560
    color: "transparent"

    onClosing: {
        close.accepted = false;
        GlobalState.showBluetoothSettings = false;
    }

    property bool bluetoothEnabled: false
    property string activeMac: ""
    property string activeName: ""
    property string activeTrusted: ""
    property string activePaired: ""
    property string selectedMac: ""
    property string selectedName: ""
    property bool selectedConnected: false
    property bool selectedTrusted: false
    property bool selectedPaired: false
    property bool hasSelection: selectedMac.length > 0
    property string statusMessage: ""
    property var devices: []
    property bool listLoading: false
    property bool scanning: false
    property bool pendingAutoScan: false
    property int autoScanRetryCount: 0
    property int emptyListStreak: 0
    property bool animateRowsOnNextLoad: true
    // pendingAction fields model transient operation state for UI feedback and recovery timers.
    property string pendingAction: ""
    property string pendingActionTarget: ""
    property string pendingPowerSyncTarget: ""
    property int powerSyncRetryCount: 0
    readonly property string missingBluetoothctlMessage: "bluetoothctl is required for Bluetooth controls."
    readonly property int actionTimeoutMs: 7000
    readonly property int powerSyncPollMs: 500
    readonly property int powerSyncMaxRetries: 14
    readonly property int openAutoScanDelayMs: 1200
    readonly property int toggleAutoScanDelayMs: 1800
    readonly property int autoScanRetryDelayMs: 900
    readonly property int autoScanMaxRetries: 12
    readonly property int scanRefreshIntervalMs: 1000
    readonly property int scanWatchdogMs: 7000
    readonly property int postScanSyncDelayMs: 1200
    readonly property int statusClearDelayMs: 3500
    readonly property int refreshIntervalMs: 12000

    function setStatusMessage(message, autoClear) {
        statusMessage = message;
        if (autoClear)
            statusClearTimer.restart();
        else
            statusClearTimer.stop();
    }

    function finishScanState() {
        scanning = false;
        GlobalState.bluetoothScanning = false;
        scanRefreshTimer.running = false;
        scanWatchdogTimer.running = false;
        listLoading = false;
    }

    function requestAutoScan() {
        pendingAutoScan = true;
        autoScanRetryCount = 0;
        autoScanRetryTimer.restart();
        btStateProc.running = true;
    }

    function beginPowerStateSync(target) {
        pendingPowerSyncTarget = target;
        powerSyncRetryCount = 0;
        powerStateSyncTimer.restart();
        btStateProc.running = true;
    }

    function finishPowerStateSync(matched) {
        const target = pendingPowerSyncTarget;
        pendingPowerSyncTarget = "";
        powerSyncRetryCount = 0;
        powerStateSyncTimer.stop();

        if (!matched) {
            setStatusMessage("Bluetooth power state did not switch to " + target + ".", true);
            refreshAll();
            return;
        }

        if (target === "on") {
            requestAutoScan();
            toggleOnAutoScanTimer.restart();
            setStatusMessage("Bluetooth turned on. Starting scan...", true);
        } else {
            pendingAutoScan = false;
            autoScanRetryCount = 0;
            autoScanRetryTimer.stop();
            toggleOnAutoScanTimer.stop();
            GlobalState.bluetoothConnected = false;
            GlobalState.bluetoothScanning = false;
            clearDeviceState();
            setStatusMessage("Bluetooth turned off.", true);
        }

        refreshAll();
    }

    function splitPipeFields(line, expectedFields) {
        const parts = line.split("|");
        while (parts.length < expectedFields)
            parts.push("");
        return parts;
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

    function sortDevicesByPriority(list) {
        list.sort(function (a, b) {
            const aConnected = a.connected === "yes";
            const bConnected = b.connected === "yes";
            if (aConnected !== bConnected)
                return aConnected ? -1 : 1;
            return a.name.localeCompare(b.name);
        });
    }

    function mergeScanOutputDevices(output) {
        if (!output)
            return;

        const lines = output.split("\n");
        const byMac = {};
        for (let i = 0; i < bluetoothMenu.devices.length; i++)
            byMac[bluetoothMenu.devices[i].mac] = bluetoothMenu.devices[i];

        let found = false;
        let addedCount = 0;
        for (let i = 0; i < lines.length; i++) {
            const line = lines[i].trim();
            if (!line)
                continue;

            let mac = "";
            let name = "";

            let match = line.match(/^\[(?:NEW|CHG)\]\s+Device\s+([0-9A-F:]{17})(?:\s+(.+))?$/i);
            if (match) {
                mac = match[1].toUpperCase();
                const tail = (match[2] || "").trim();
                // Skip attribute-only change lines like "RSSI:" / "TxPower:".
                if (tail && tail.indexOf(":") === -1)
                    name = tail;
            } else {
                match = line.match(/^Device\s+([0-9A-F:]{17})(?:\s+(.+))?$/i);
                if (!match)
                    continue;
                mac = match[1].toUpperCase();
                name = (match[2] || "").trim();
            }

            if (!byMac[mac]) {
                byMac[mac] = {
                    mac: mac,
                    name: name || mac,
                    connected: "no",
                    trusted: "no",
                    paired: "no"
                };
                found = true;
                addedCount++;
            } else if (name && byMac[mac].name === mac) {
                byMac[mac].name = name;
            }
        }

        if (!found)
            return;

        const merged = Object.values(byMac);
        bluetoothMenu.sortDevicesByPriority(merged);
        bluetoothMenu.devices = merged;
        bluetoothMenu.emptyListStreak = 0;
    }

    function refreshAll() {
        listLoading = true;
        btStateProc.running = true;
        btListProc.running = true;
    }

    function clearDeviceState() {
        devices = [];
        emptyListStreak = 0;
        activeMac = "";
        activeName = "";
        activeTrusted = "";
        activePaired = "";
        clearSelection();
    }

    function clearSelection() {
        selectedMac = "";
        selectedName = "";
        selectedConnected = false;
        selectedTrusted = false;
        selectedPaired = false;
    }

    function pairSelectedDevice() {
        if (!selectedMac || selectedPaired)
            return;

        actionProc.command = ["stratum-cli", "bluetooth", "pair", selectedMac];
        pendingAction = "pair";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Pairing with " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function connectSelectedDevice() {
        if (!selectedMac || selectedConnected)
            return;

        actionProc.command = ["stratum-cli", "bluetooth", "connect", selectedMac];
        pendingAction = "connect";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Connecting to " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function disconnectCurrentDevice() {
        const targetMac = activeMac || selectedMac;
        if (!targetMac)
            return;

        const label = activeName || selectedName || targetMac;
        actionProc.command = ["stratum-cli", "bluetooth", "disconnect", targetMac];
        pendingAction = "disconnect";
        pendingActionTarget = label;
        setStatusMessage("Disconnecting " + label + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function forgetSelectedDevice() {
        if (!selectedMac)
            return;

        actionProc.command = ["stratum-cli", "bluetooth", "forget", selectedMac];
        pendingAction = "forget";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Removing " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function trustSelectedDevice() {
        if (!selectedMac || selectedTrusted)
            return;

        actionProc.command = ["stratum-cli", "bluetooth", "trust", selectedMac];
        pendingAction = "trust";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Trusting " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function untrustSelectedDevice() {
        if (!selectedMac || !selectedTrusted)
            return;

        actionProc.command = ["stratum-cli", "bluetooth", "untrust", selectedMac];
        pendingAction = "untrust";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Removing trust for " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function toggleBluetoothPower() {
        const target = bluetoothEnabled ? "off" : "on";
        actionProc.command = ["stratum-cli", "bluetooth", "power", target];
        pendingAction = "toggle";
        pendingActionTarget = target;
        setStatusMessage(bluetoothEnabled ? "Turning Bluetooth off..." : "Turning Bluetooth on...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function startScan() {
        if (scanning)
            return;

        scanProc.command = ["stratum-cli", "bluetooth", "scan"];
        scanning = true;
        GlobalState.bluetoothScanning = true;
        setStatusMessage("Scanning for devices...", false);
        listLoading = true;
        scanRefreshTimer.running = true;
        scanWatchdogTimer.running = true;
        btListProc.running = true;
        scanProc.running = true;
    }

    Process {
        id: btStateProc
        command: ["stratum-cli", "bluetooth", "state"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = bluetoothMenu.parseCliJson(result);
                if (!payload || payload.ok !== true) {
                    bluetoothMenu.bluetoothEnabled = false;
                    GlobalState.bluetoothPowered = false;
                    GlobalState.bluetoothConnected = false;
                    GlobalState.bluetoothScanning = false;
                    bluetoothMenu.setStatusMessage(payload && payload.error ? String(payload.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                    return;
                }

                bluetoothMenu.bluetoothEnabled = String(payload.powered || "no") === "yes";
                GlobalState.bluetoothPowered = bluetoothMenu.bluetoothEnabled;

                if (bluetoothMenu.pendingPowerSyncTarget) {
                    const expectedOn = bluetoothMenu.pendingPowerSyncTarget === "on";
                    if (bluetoothMenu.bluetoothEnabled === expectedOn) {
                        bluetoothMenu.finishPowerStateSync(true);
                        return;
                    }
                }

                if (!bluetoothMenu.bluetoothEnabled) {
                    GlobalState.bluetoothConnected = false;
                    GlobalState.bluetoothScanning = false;
                    const poweringOn = bluetoothMenu.pendingAction === "toggle" && bluetoothMenu.pendingActionTarget === "on";
                    if (!bluetoothMenu.pendingAutoScan && !bluetoothMenu.scanning && !poweringOn)
                        bluetoothMenu.clearDeviceState();
                }

                if (bluetoothMenu.visible && bluetoothMenu.pendingAutoScan) {
                    if (bluetoothMenu.bluetoothEnabled && !bluetoothMenu.scanning) {
                        bluetoothMenu.pendingAutoScan = false;
                        bluetoothMenu.autoScanRetryCount = 0;
                        bluetoothMenu.autoScanRetryTimer.stop();
                        bluetoothMenu.startScan();
                    } else if (!bluetoothMenu.bluetoothEnabled && !bluetoothMenu.scanning) {
                        bluetoothMenu.autoScanRetryTimer.restart();
                    }
                }
            }
        }
    }

    Process {
        id: btListProc
        command: ["stratum-cli", "bluetooth", "list"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (!bluetoothMenu.scanning)
                    bluetoothMenu.listLoading = false;

                const payload = bluetoothMenu.parseCliJson(result);
                if (!payload || payload.ok !== true) {
                    bluetoothMenu.devices = [];
                    bluetoothMenu.setStatusMessage(payload && payload.error ? String(payload.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                    return;
                }

                const rows = Array.isArray(payload.devices) ? payload.devices : [];
                const parsed = [];

                for (let i = 0; i < rows.length; i++) {
                    const row = rows[i] || {};
                    const mac = String(row.mac || "").trim();
                    if (!mac)
                        continue;

                    parsed.push({
                        mac: mac,
                        name: String(row.name || "").trim() || mac,
                        connected: String(row.connected || "no").trim(),
                        trusted: String(row.trusted || "no").trim(),
                        paired: String(row.paired || "yes").trim()
                    });
                }

                if (parsed.length === 0) {
                    bluetoothMenu.emptyListStreak += 1;
                    if (bluetoothMenu.scanning)
                        return;
                    // bluetoothctl can briefly return no rows during adapter transitions.
                    if (bluetoothMenu.emptyListStreak < 3 && bluetoothMenu.devices.length > 0)
                        return;
                } else {
                    bluetoothMenu.emptyListStreak = 0;
                }

                bluetoothMenu.sortDevicesByPriority(parsed);

                bluetoothMenu.devices = parsed;
                if (bluetoothMenu.animateRowsOnNextLoad)
                    bluetoothMenu.animateRowsOnNextLoad = false;

                let active = null;
                for (let i = 0; i < parsed.length; i++) {
                    if (parsed[i].connected === "yes") {
                        active = parsed[i];
                        break;
                    }
                }

                if (active) {
                    bluetoothMenu.activeMac = active.mac;
                    bluetoothMenu.activeName = active.name;
                    bluetoothMenu.activeTrusted = active.trusted;
                    bluetoothMenu.activePaired = active.paired;
                    GlobalState.bluetoothConnected = true;
                } else {
                    bluetoothMenu.activeMac = "";
                    bluetoothMenu.activeName = "";
                    bluetoothMenu.activeTrusted = "";
                    bluetoothMenu.activePaired = "";
                    GlobalState.bluetoothConnected = false;
                }

                let selectedFound = false;
                for (let i = 0; i < parsed.length; i++) {
                    if (parsed[i].mac === bluetoothMenu.selectedMac) {
                        bluetoothMenu.selectedName = parsed[i].name;
                        bluetoothMenu.selectedConnected = parsed[i].connected == "yes";
                        bluetoothMenu.selectedTrusted = parsed[i].trusted == "yes";
                        bluetoothMenu.selectedPaired = parsed[i].paired == "yes";
                        selectedFound = true;
                        break;
                    }
                }

                if (!selectedFound)
                    bluetoothMenu.clearSelection();
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                bluetoothMenu.actionWatchdogTimer.stop();

                const payload = bluetoothMenu.parseCliJson(result);
                const message = payload ? String(payload.output || payload.error || "") : result;

                const {
                    pendingAction: action,
                    pendingActionTarget: target
                } = bluetoothMenu;
                const lowerMsg = (message || "").toLowerCase();

                if (!payload?.ok) {
                    const errorMsg = payload?.error ? String(payload.error) : bluetoothMenu.missingBluetoothctlMessage;
                    bluetoothMenu.setStatusMessage(errorMsg, true);
                } else if (lowerMsg.includes("failed") || lowerMsg.includes("error")) {
                    bluetoothMenu.setStatusMessage(message, true);
                } else if (action === "toggle") {
                    bluetoothMenu.beginPowerStateSync(target);
                } else {
                    const successMessages = {
                        pair: `Paired with ${target}.`,
                        connect: `Connected to ${target}.`,
                        disconnect: `Disconnected ${target}.`,
                        forget: `Removed ${target}.`,
                        trust: `Trusted ${target}.`,
                        untrust: `Untrusted ${target}.`
                    };

                    let finalMessage = successMessages[action];

                    if (!finalMessage) {
                        const actionLabel = action ? action.charAt(0).toUpperCase() + action.slice(1) : "Action";
                        finalMessage = `${actionLabel}${target ? ` ${target}` : ""} completed.`;
                    }

                    bluetoothMenu.setStatusMessage(finalMessage, true);
                }

                bluetoothMenu.pendingAction = "";
                bluetoothMenu.pendingActionTarget = "";
                if (!bluetoothMenu.pendingPowerSyncTarget)
                    bluetoothMenu.refreshAll();
            }
        }
    }

    Timer {
        id: actionWatchdogTimer
        interval: bluetoothMenu.actionTimeoutMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.pendingAction)
                return;

            const action = bluetoothMenu.pendingAction;
            const target = bluetoothMenu.pendingActionTarget;
            bluetoothMenu.pendingAction = "";
            bluetoothMenu.pendingActionTarget = "";

            if (action === "toggle") {
                bluetoothMenu.setStatusMessage("Bluetooth power toggle timed out. Syncing state...", true);
                bluetoothMenu.beginPowerStateSync(target);
            } else {
                const actionLabel = action.charAt(0).toUpperCase() + action.slice(1);
                const targetLabel = target ? " " + target : "";
                bluetoothMenu.setStatusMessage(actionLabel + targetLabel + " timed out.", true);
                bluetoothMenu.refreshAll();
            }
        }
    }

    Timer {
        id: powerStateSyncTimer
        interval: bluetoothMenu.powerSyncPollMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.pendingPowerSyncTarget)
                return;

            bluetoothMenu.powerSyncRetryCount += 1;
            if (bluetoothMenu.powerSyncRetryCount <= bluetoothMenu.powerSyncMaxRetries) {
                btStateProc.running = true;
                powerStateSyncTimer.restart();
            } else {
                bluetoothMenu.finishPowerStateSync(false);
            }
        }
    }

    Process {
        id: scanProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();

                const payload = bluetoothMenu.parseCliJson(result);
                const outputText = payload ? String(payload.output || "") : result;

                bluetoothMenu.mergeScanOutputDevices(outputText);
                bluetoothMenu.finishScanState();

                if (!payload || payload.ok !== true) {
                    bluetoothMenu.setStatusMessage(payload && payload.error ? String(payload.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                } else if (outputText.length > 0 && outputText.toLowerCase().indexOf("failed") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan failed: " + outputText, true);
                } else if (outputText.length > 0 && outputText.toLowerCase().indexOf("error") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan error: " + outputText, true);
                } else {
                    bluetoothMenu.setStatusMessage("Scan completed.", true);
                }

                bluetoothMenu.refreshAll();
                bluetoothMenu.postScanSyncTimer.restart();
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: GlobalState.showBluetoothSettings = false
    }

    onVisibleChanged: {
        if (visible) {
            animateRowsOnNextLoad = true;
            requestAutoScan();
            openAutoScanTimer.restart();
            refreshAll();
            refreshTimer.running = true;
        } else {
            refreshTimer.running = false;
            pendingAutoScan = false;
            autoScanRetryCount = 0;
            autoScanRetryTimer.stop();
            pendingPowerSyncTarget = "";
            powerSyncRetryCount = 0;
            powerStateSyncTimer.stop();
            openAutoScanTimer.stop();
            toggleOnAutoScanTimer.stop();
            if (scanning) {
                finishScanState();
            }
        }
    }

    Timer {
        id: openAutoScanTimer
        interval: bluetoothMenu.openAutoScanDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.visible || bluetoothMenu.scanning)
                return;

            btStateProc.running = true;
            if (bluetoothMenu.bluetoothEnabled)
                bluetoothMenu.startScan();
        }
    }

    Timer {
        id: toggleOnAutoScanTimer
        interval: bluetoothMenu.toggleAutoScanDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.visible || bluetoothMenu.scanning)
                return;

            btStateProc.running = true;
            if (bluetoothMenu.bluetoothEnabled)
                bluetoothMenu.startScan();
        }
    }

    Timer {
        id: autoScanRetryTimer
        interval: bluetoothMenu.autoScanRetryDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.visible || !bluetoothMenu.pendingAutoScan || bluetoothMenu.scanning)
                return;

            if (bluetoothMenu.bluetoothEnabled) {
                bluetoothMenu.pendingAutoScan = false;
                bluetoothMenu.autoScanRetryCount = 0;
                bluetoothMenu.startScan();
                return;
            }

            bluetoothMenu.autoScanRetryCount += 1;
            if (bluetoothMenu.autoScanRetryCount <= bluetoothMenu.autoScanMaxRetries) {
                btStateProc.running = true;
                autoScanRetryTimer.restart();
            } else {
                bluetoothMenu.pendingAutoScan = false;
                bluetoothMenu.autoScanRetryCount = 0;
            }
        }
    }

    Timer {
        id: scanRefreshTimer
        interval: bluetoothMenu.scanRefreshIntervalMs
        repeat: true
        running: false
        onTriggered: {
            if (bluetoothMenu.scanning && !btListProc.running)
                btListProc.running = true;
        }
    }

    Timer {
        id: scanWatchdogTimer
        interval: bluetoothMenu.scanWatchdogMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.scanning)
                return;

            bluetoothMenu.finishScanState();
            bluetoothMenu.setStatusMessage("Scan completed.", true);
            bluetoothMenu.refreshAll();
            bluetoothMenu.postScanSyncTimer.restart();
        }
    }

    Timer {
        id: postScanSyncTimer
        interval: bluetoothMenu.postScanSyncDelayMs
        repeat: false
        running: false
        onTriggered: bluetoothMenu.refreshAll()
    }

    Timer {
        id: statusClearTimer
        interval: bluetoothMenu.statusClearDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.scanning)
                bluetoothMenu.statusMessage = "";
        }
    }

    Timer {
        id: refreshTimer
        interval: bluetoothMenu.refreshIntervalMs
        repeat: true
        running: false
        onTriggered: bluetoothMenu.refreshAll()
    }

    Rectangle {
        id: menuCard
        anchors.fill: parent
        color: Theme.palette.bgMain
        radius: 12

        MouseArea {
            anchors.fill: parent
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 12

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "Bluetooth"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 18
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 98
                    radius: 6
                    color: btToggleMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: bluetoothMenu.pendingAction === "toggle" ? "󰔟" : (bluetoothMenu.bluetoothEnabled ? "󰂯" : "󰂲")
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 13
                        }

                        Text {
                            text: bluetoothMenu.pendingAction === "toggle" ? "Working" : (bluetoothMenu.bluetoothEnabled ? "On" : "Off")
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 12
                            font.bold: true
                        }
                    }

                    MouseArea {
                        id: btToggleMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: bluetoothMenu.pendingAction !== "toggle"
                        onClicked: bluetoothMenu.toggleBluetoothPower()
                    }
                }

                Rectangle {
                    id: closeButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    radius: 6
                    color: closeMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.palette.error
                        font.family: Theme.palette.font
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: GlobalState.showBluetoothSettings = false
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 64
                color: Theme.palette.bgWidget
                radius: 8

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: 0

                    RowLayout {
                        Layout.fillWidth: true

                        Text {
                            text: bluetoothMenu.activeName ? "Connected to " + bluetoothMenu.activeName : "No active Bluetooth device"
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 13
                            font.bold: true
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Rectangle {
                            id: disconnectButton
                            property bool isEnabled: bluetoothMenu.activeMac.length > 0
                            Layout.preferredHeight: 30
                            Layout.preferredWidth: 38
                            radius: 6
                            color: !isEnabled ? Theme.palette.bgDark : (topDisconnectMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget)

                            StyledIconToolTip {
                                visible: topDisconnectMouse.containsMouse
                                text: "Disconnect"
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰂲"
                                color: Theme.palette.textMain
                                font.family: Theme.palette.font
                                font.pixelSize: 20
                            }

                            MouseArea {
                                id: topDisconnectMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: bluetoothMenu.disconnectCurrentDevice()
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true

                        Text {
                            text: "MAC: " + (bluetoothMenu.activeMac ? bluetoothMenu.activeMac : "N/A")
                            color: bluetoothMenu.activeMac ? Theme.palette.secondary : Theme.palette.tertiary
                            font.family: Theme.palette.font
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "•"
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 10
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "Trusted: " + (bluetoothMenu.activeTrusted ? bluetoothMenu.activeTrusted : "N/A")
                            color: bluetoothMenu.activeTrusted ? Theme.palette.success : Theme.palette.error
                            font.family: Theme.palette.font
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "•"
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 10
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "Paired: " + (bluetoothMenu.activePaired ? bluetoothMenu.activePaired : "N/A")
                            color: bluetoothMenu.activePaired ? Theme.palette.success : Theme.palette.error
                            font.family: Theme.palette.font
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Rectangle {
                    id: refreshButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    radius: 6
                    color: refreshMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    StyledIconToolTip {
                        visible: refreshMouse.containsMouse
                        text: "Refresh"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: "󰑐"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: refreshMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: bluetoothMenu.refreshAll()
                    }
                }

                Rectangle {
                    id: scanButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    radius: 6
                    color: scanMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    StyledIconToolTip {
                        visible: scanMouse.containsMouse
                        text: "Scan"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: ""
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 13
                    }

                    MouseArea {
                        id: scanMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: bluetoothMenu.bluetoothEnabled
                        onClicked: bluetoothMenu.startScan()
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: bluetoothMenu.devices.length + " devices"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                }

                RowLayout {
                    spacing: 6
                    visible: bluetoothMenu.listLoading

                    Rectangle {
                        implicitWidth: 8
                        implicitHeight: 8
                        radius: 4
                        color: Theme.palette.success

                        SequentialAnimation on opacity {
                            running: bluetoothMenu.listLoading
                            loops: Animation.Infinite

                            NumberAnimation {
                                from: 0.3
                                to: 1
                                duration: 420
                                easing.type: Easing.OutCubic
                            }

                            NumberAnimation {
                                from: 1
                                to: 0.3
                                duration: 420
                                easing.type: Easing.InCubic
                            }
                        }
                    }

                    Text {
                        text: "Loading"
                        color: Theme.palette.success
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: bluetoothMenu.hasSelection ? 10 : 0

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.palette.bgMain
                    radius: 8

                    ScrollView {
                        id: bluetoothDeviceScroll
                        anchors.fill: parent
                        anchors.margins: 8
                        clip: true
                        contentWidth: availableWidth

                        Column {
                            width: bluetoothDeviceScroll.availableWidth
                            spacing: 6

                            Repeater {
                                model: bluetoothMenu.devices

                                delegate: Rectangle {
                                    id: deviceRow
                                    required property var modelData
                                    required property var index
                                    property bool shouldAnimateOnCreate: bluetoothMenu.animateRowsOnNextLoad

                                    width: parent.width
                                    height: 56
                                    radius: 6
                                    color: bluetoothMenu.selectedMac === modelData.mac ? Theme.palette.bgHover : Theme.palette.bgWidget
                                    border.color: modelData.connected === "yes" ? Theme.palette.borderActive : Theme.palette.borderInactive
                                    border.width: 1
                                    opacity: 1

                                    SequentialAnimation {
                                        id: rowFadeIn
                                        running: false

                                        PauseAnimation {
                                            duration: Math.min(360, index * 36)
                                        }

                                        NumberAnimation {
                                            target: deviceRow
                                            property: "opacity"
                                            from: 0
                                            to: 1
                                            duration: 170
                                            easing.type: Easing.OutCubic
                                        }
                                    }

                                    Component.onCompleted: {
                                        if (deviceRow.shouldAnimateOnCreate) {
                                            deviceRow.opacity = 0;
                                            rowFadeIn.restart();
                                        } else {
                                            deviceRow.opacity = 1;
                                        }
                                    }

                                    onModelDataChanged: {
                                        // Ensure reused delegates never stay hidden after list refreshes.
                                        if (!rowFadeIn.running)
                                            deviceRow.opacity = 1;
                                    }

                                    Behavior on color {
                                        ColorAnimation {
                                            duration: 160
                                        }
                                    }

                                    Behavior on border.color {
                                        ColorAnimation {
                                            duration: 160
                                        }
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: {
                                            if (bluetoothMenu.selectedMac === modelData.mac) {
                                                bluetoothMenu.clearSelection();
                                            } else {
                                                bluetoothMenu.selectedMac = modelData.mac;
                                                bluetoothMenu.selectedName = modelData.name;
                                                bluetoothMenu.selectedConnected = modelData.connected == "yes";
                                                bluetoothMenu.selectedTrusted = modelData.trusted == "yes";
                                                bluetoothMenu.selectedPaired = modelData.paired == "yes";
                                            }
                                        }
                                    }

                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.margins: 8
                                        spacing: 8

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Text {
                                                text: modelData.name
                                                color: Theme.palette.textMain
                                                font.family: Theme.palette.font
                                                font.pixelSize: 12
                                                elide: Text.ElideRight
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true

                                                Text {
                                                    text: modelData.mac
                                                    color: bluetoothMenu.selectedMac === modelData.mac ? Theme.palette.secondary : Theme.palette.tertiary
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    elide: Text.ElideRight
                                                }

                                                Text {
                                                    text: "•"
                                                    color: Theme.palette.textMain
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    elide: Text.ElideRight
                                                }

                                                Text {
                                                    text: modelData.connected === "yes" ? "Connected" : "Disconnected"
                                                    color: modelData.connected === "yes" ? Theme.palette.success : Theme.palette.error
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    elide: Text.ElideRight
                                                }

                                                Text {
                                                    text: "•"
                                                    color: Theme.palette.textMain
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    elide: Text.ElideRight
                                                }

                                                Text {
                                                    text: modelData.trusted === "yes" ? "Trusted" : "Untrusted"
                                                    color: modelData.trusted === "yes" ? Theme.palette.success : Theme.palette.error
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    elide: Text.ElideRight
                                                }
                                            }
                                        }

                                        Text {
                                            text: "󰅂"
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 20
                                            Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Rectangle {
                    Layout.preferredWidth: bluetoothMenu.hasSelection ? 250 : 0
                    Layout.fillHeight: true
                    color: Theme.palette.bgWidget
                    radius: 8
                    border.color: Theme.palette.borderActive
                    border.width: 1
                    clip: true

                    Behavior on Layout.preferredWidth {
                        NumberAnimation {
                            duration: 180
                            easing.type: Easing.OutCubic
                        }
                    }

                    Behavior on opacity {
                        NumberAnimation {
                            duration: 140
                        }
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 8

                        RowLayout {
                            Layout.fillWidth: true

                            Text {
                                text: "Selected Device"
                                color: Theme.palette.textMuted
                                font.family: Theme.palette.font
                                font.pixelSize: 12
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            Rectangle {
                                Layout.preferredWidth: 30
                                Layout.preferredHeight: 28
                                radius: 5
                                color: sideCloseMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                                RowLayout {
                                    anchors.centerIn: parent
                                    spacing: 4

                                    Text {
                                        text: "󰅖"
                                        color: Theme.palette.error
                                        font.family: Theme.palette.font
                                        font.pixelSize: 16
                                    }
                                }

                                MouseArea {
                                    id: sideCloseMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: bluetoothMenu.clearSelection()
                                }
                            }
                        }

                        Text {
                            text: bluetoothMenu.selectedName
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 14
                            font.bold: true
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "MAC: " + bluetoothMenu.selectedMac
                            color: Theme.palette.secondary
                            font.family: Theme.palette.font
                            font.pixelSize: 11
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 5

                            Text {
                                text: bluetoothMenu.selectedConnected ? "Connected" : "Disconnected"
                                color: bluetoothMenu.selectedConnected ? Theme.palette.success : Theme.palette.error
                                font.family: Theme.palette.font
                                font.pixelSize: 11
                                elide: Text.ElideRight
                            }

                            Text {
                                text: "•"
                                color: Theme.palette.textMain
                                font.family: Theme.palette.font
                                font.pixelSize: 11
                            }

                            Text {
                                text: bluetoothMenu.selectedTrusted ? "Trusted" : "Untrusted"
                                color: bluetoothMenu.selectedTrusted ? Theme.palette.success : Theme.palette.error
                                font.family: Theme.palette.font
                                font.pixelSize: 11
                                elide: Text.ElideRight
                            }

                            Text {
                                text: "•"
                                color: Theme.palette.textMain
                                font.family: Theme.palette.font
                                font.pixelSize: 11
                            }

                            Text {
                                Layout.fillWidth: true
                                text: bluetoothMenu.selectedPaired ? "Paired" : "Paired"
                                color: bluetoothMenu.selectedPaired ? Theme.palette.success : Theme.palette.error
                                font.family: Theme.palette.font
                                font.pixelSize: 11
                                elide: Text.ElideRight
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Theme.palette.secondary
                        }

                        Rectangle {
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: pairMainMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                            border.color: Theme.palette.borderInactive
                            border.width: 1

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: bluetoothMenu.selectedPaired ? "" : "󰌹"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: bluetoothMenu.selectedPaired ? "Remove" : "Pair"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: pairMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.enabled
                                onClicked: bluetoothMenu.selectedPaired ? bluetoothMenu.removeSelectedDevice() : bluetoothMenu.pairSelectedDevice()
                            }
                        }

                        Rectangle {
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: connectMainMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                            border.color: Theme.palette.borderInactive
                            border.width: 1

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰖩"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: bluetoothMenu.selectedConnected ? "Disconnect" : "Connect"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: connectMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: bluetoothMenu.selectedConnected ? bluetoothMenu.disconnectCurrentDevice() : bluetoothMenu.connectSelectedDevice()
                            }
                        }

                        Rectangle {
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: removeMainMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget
                            border.color: Theme.palette.borderInactive
                            border.width: 1

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: bluetoothMenu.selectedTrusted ? "󰦞" : "󰕥"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: bluetoothMenu.selectedTrusted ? "Untrust" : "Trust"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: removeMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: bluetoothMenu.selectedTrusted ? bluetoothMenu.untrustSelectedDevice() : bluetoothMenu.trustSelectedDevice()
                            }
                        }

                        Item {
                            Layout.fillHeight: true
                        }
                    }
                }
            }

            Text {
                Layout.fillWidth: true
                text: bluetoothMenu.statusMessage
                color: Theme.palette.tertiary
                font.family: Theme.palette.font
                font.pixelSize: 11
                wrapMode: Text.Wrap
                visible: text.length > 0
            }
        }
    }
}
