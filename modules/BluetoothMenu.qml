import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Quickshell
import Quickshell.Io

import "../theme"
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
    property string selectedConnected: ""
    property string selectedTrusted: ""
    property string selectedPaired: ""
    property bool hasSelection: selectedMac.length > 0
    property string statusMessage: ""
    property var devices: []
    property bool listLoading: false
    property bool scanning: false
    property bool pendingAutoScan: false
    property int autoScanRetryCount: 0
    property int emptyListStreak: 0
    property bool animateRowsOnNextLoad: true
    property string pendingAction: ""
    property string pendingActionTarget: ""
    property string pendingPowerSyncTarget: ""
    property int powerSyncRetryCount: 0

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

    function sortDevicesByPriority(list) {
        list.sort(function(a, b) {
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
        selectedConnected = "";
        selectedTrusted = "";
        selectedPaired = "";
    }

    function pairSelectedDevice() {
        if (!selectedMac || selectedPaired === "yes")
            return;

        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "pair", selectedMac];
        pendingAction = "pair";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Pairing with " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function connectSelectedDevice() {
        if (!selectedMac || selectedConnected === "yes")
            return;

        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "connect", selectedMac];
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
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "disconnect", targetMac];
        pendingAction = "disconnect";
        pendingActionTarget = label;
        setStatusMessage("Disconnecting " + label + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function forgetSelectedDevice() {
        if (!selectedMac)
            return;

        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "forget", selectedMac];
        pendingAction = "forget";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Removing " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function toggleBluetoothPower() {
        const target = bluetoothEnabled ? "off" : "on";
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "power", target];
        pendingAction = "toggle";
        pendingActionTarget = target;
        setStatusMessage(bluetoothEnabled ? "Turning Bluetooth off..." : "Turning Bluetooth on...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function startScan() {
        if (scanning)
            return;

        scanProc.command = ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "scan"];
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
        command: ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "state"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result.startsWith("__ERROR__|")) {
                    bluetoothMenu.bluetoothEnabled = false;
                    GlobalState.bluetoothPowered = false;
                    GlobalState.bluetoothConnected = false;
                    GlobalState.bluetoothScanning = false;
                    bluetoothMenu.setStatusMessage("bluetoothctl is required for Bluetooth controls.", true);
                    return;
                }
                bluetoothMenu.bluetoothEnabled = result === "yes";
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
        command: ["sh", Quickshell.shellDir + "/scripts/bluetooth_menu.sh", "list"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (!bluetoothMenu.scanning)
                    bluetoothMenu.listLoading = false;

                if (result.startsWith("__ERROR__|")) {
                    bluetoothMenu.devices = [];
                    bluetoothMenu.setStatusMessage("bluetoothctl is required for Bluetooth controls.", true);
                    return;
                }

                const lines = result.length > 0 ? result.split("\n") : [];
                const parsed = [];

                for (let i = 0; i < lines.length; i++) {
                    const cols = bluetoothMenu.splitPipeFields(lines[i], 5);
                    const mac = cols[0].trim();
                    if (!mac)
                        continue;

                    parsed.push({
                        mac: mac,
                        name: cols[1].trim() || mac,
                        connected: cols[2].trim() || "no",
                        trusted: cols[3].trim() || "no",
                        paired: cols[4].trim() || "yes"
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
                        bluetoothMenu.selectedConnected = parsed[i].connected;
                        bluetoothMenu.selectedTrusted = parsed[i].trusted;
                        bluetoothMenu.selectedPaired = parsed[i].paired;
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

                if (result.startsWith("__ERROR__|")) {
                    bluetoothMenu.setStatusMessage("bluetoothctl is required for Bluetooth controls.", true);
                } else if (result.length > 0 && result.toLowerCase().indexOf("failed") !== -1) {
                    bluetoothMenu.setStatusMessage(result, true);
                } else if (result.length > 0 && result.toLowerCase().indexOf("error") !== -1) {
                    bluetoothMenu.setStatusMessage(result, true);
                } else if (bluetoothMenu.pendingAction === "pair") {
                    bluetoothMenu.setStatusMessage("Paired with " + bluetoothMenu.pendingActionTarget + ".", true);
                } else if (bluetoothMenu.pendingAction === "connect") {
                    bluetoothMenu.setStatusMessage("Connected to " + bluetoothMenu.pendingActionTarget + ".", true);
                } else if (bluetoothMenu.pendingAction === "disconnect") {
                    bluetoothMenu.setStatusMessage("Disconnected " + bluetoothMenu.pendingActionTarget + ".", true);
                } else if (bluetoothMenu.pendingAction === "forget") {
                    bluetoothMenu.setStatusMessage("Removed " + bluetoothMenu.pendingActionTarget + ".", true);
                } else if (bluetoothMenu.pendingAction === "toggle") {
                    bluetoothMenu.beginPowerStateSync(bluetoothMenu.pendingActionTarget);
                } else {
                    const actionLabel = bluetoothMenu.pendingAction ? (bluetoothMenu.pendingAction.charAt(0).toUpperCase() + bluetoothMenu.pendingAction.slice(1)) : "Action";
                    const targetLabel = bluetoothMenu.pendingActionTarget ? " " + bluetoothMenu.pendingActionTarget : "";
                    bluetoothMenu.setStatusMessage(actionLabel + targetLabel + " completed.", true);
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
        interval: 7000
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
        interval: 500
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.pendingPowerSyncTarget)
                return;

            bluetoothMenu.powerSyncRetryCount += 1;
            if (bluetoothMenu.powerSyncRetryCount <= 14) {
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
                bluetoothMenu.mergeScanOutputDevices(result);
                bluetoothMenu.finishScanState();

                if (result.startsWith("__ERROR__|")) {
                    bluetoothMenu.setStatusMessage("bluetoothctl is required for Bluetooth controls.", true);
                } else if (result.length > 0 && result.toLowerCase().indexOf("failed") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan failed: " + result, true);
                } else if (result.length > 0 && result.toLowerCase().indexOf("error") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan error: " + result, true);
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
        interval: 1200
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
        interval: 1800
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
        interval: 900
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
            if (bluetoothMenu.autoScanRetryCount <= 12) {
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
        interval: 1000
        repeat: true
        running: false
        onTriggered: {
            if (bluetoothMenu.scanning && !btListProc.running)
                btListProc.running = true;
        }
    }

    Timer {
        id: scanWatchdogTimer
        interval: 7000
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
        interval: 1200
        repeat: false
        running: false
        onTriggered: bluetoothMenu.refreshAll()
    }

    Timer {
        id: statusClearTimer
        interval: 3500
        repeat: false
        running: false
        onTriggered: {
            if (!bluetoothMenu.scanning)
                bluetoothMenu.statusMessage = "";
        }
    }

    Timer {
        id: refreshTimer
        interval: 12000
        repeat: true
        running: false
        onTriggered: bluetoothMenu.refreshAll()
    }

    Rectangle {
        id: menuCard
        anchors.fill: parent
        color: Theme.background
        border.width: 1
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
                    color: Theme.text
                    font.family: Theme.font
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
                    color: btToggleMouse.containsMouse ? "#263244" : "#1b2333"
                    border.color: Theme.grey
                    border.width: 1

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            id: toggleIcon
                            text: bluetoothMenu.pendingAction === "toggle" ? "󰔟" : (bluetoothMenu.bluetoothEnabled ? "󰂯" : "󰂲")
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 13
                        }

                        Text {
                            text: bluetoothMenu.pendingAction === "toggle" ? "Working" : (bluetoothMenu.bluetoothEnabled ? "On" : "Off")
                            color: Theme.text
                            font.family: Theme.font
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
                    color: closeMouse.containsMouse ? "#3a1f27" : "#2b1720"
                    border.color: Theme.grey
                    border.width: 1

                    StyledIconToolTip {
                        visible: closeMouse.containsMouse
                        text: "Close"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.text
                        font.family: Theme.font
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
                Layout.preferredHeight: 124
                color: Theme.background
                radius: 8
                border.color: Theme.grey
                border.width: 1

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: 6

                    RowLayout {
                        Layout.fillWidth: true

                        Text {
                            text: bluetoothMenu.activeName ? "Connected to " + bluetoothMenu.activeName : "No active Bluetooth device"
                            color: Theme.text
                            font.family: Theme.font
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
                            color: !isEnabled ? "#23232d" : (topDisconnectMouse.containsMouse ? "#3a1f27" : "#2b1720")
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.5

                            StyledIconToolTip {
                                visible: topDisconnectMouse.containsMouse
                                text: "Disconnect"
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰖪"
                                color: Theme.text
                                font.family: Theme.font
                                font.pixelSize: 12
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

                    Text {
                        text: "MAC: " + (bluetoothMenu.activeMac ? bluetoothMenu.activeMac : "N/A") + "  •  Trusted: " + (bluetoothMenu.activeTrusted ? bluetoothMenu.activeTrusted : "N/A") + "  •  Paired: " + (bluetoothMenu.activePaired ? bluetoothMenu.activePaired : "N/A")
                        color: Theme.hover
                        font.family: Theme.font
                        font.pixelSize: 12
                        elide: Text.ElideRight
                        Layout.fillWidth: true
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
                    color: refreshMouse.containsMouse ? "#243126" : "#19261b"
                    border.color: Theme.grey
                    border.width: 1

                    StyledIconToolTip {
                        visible: refreshMouse.containsMouse
                        text: "Refresh"
                    }

                    Text {
                        anchors.centerIn: parent
                        text: "󰑐"
                        color: Theme.text
                        font.family: Theme.font
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
                    Layout.preferredWidth: 76
                    radius: 6
                    color: scanMouse.containsMouse ? "#243126" : "#19261b"
                    border.color: Theme.grey
                    border.width: 1
                    opacity: bluetoothMenu.bluetoothEnabled ? 1.0 : 0.6

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: "󰒓"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 13
                        }

                        Text {
                            text: "Scan"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 11
                            font.bold: true
                        }
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
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                }

                RowLayout {
                    spacing: 6
                    visible: bluetoothMenu.listLoading

                    Rectangle {
                        width: 8
                        height: 8
                        radius: 4
                        color: Theme.activeWs
                        opacity: 0.35

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
                        color: Theme.activeWs
                        font.family: Theme.font
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
                    color: Theme.background
                    radius: 8
                    border.color: Theme.grey
                    border.width: 1

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
                                    required property var modelData
                                    id: deviceRow
                                    property bool shouldAnimateOnCreate: bluetoothMenu.animateRowsOnNextLoad

                                    width: parent.width
                                    height: 56
                                    radius: 6
                                    color: bluetoothMenu.selectedMac === modelData.mac ? "#1d2434" : Theme.background
                                    border.color: modelData.connected === "yes" ? Theme.activeWs : Theme.grey
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
                                                bluetoothMenu.selectedConnected = modelData.connected;
                                                bluetoothMenu.selectedTrusted = modelData.trusted;
                                                bluetoothMenu.selectedPaired = modelData.paired;
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
                                                color: Theme.text
                                                font.family: Theme.font
                                                font.pixelSize: 12
                                                elide: Text.ElideRight
                                            }

                                            Text {
                                                text: modelData.mac + "  •  " + (modelData.connected === "yes" ? "Connected" : "Disconnected") + "  •  Trusted " + modelData.trusted
                                                color: Theme.hover
                                                font.family: Theme.font
                                                font.pixelSize: 10
                                                elide: Text.ElideRight
                                            }
                                        }

                                        Text {
                                            text: "󰅂"
                                            color: Theme.hover
                                            font.family: Theme.font
                                            font.pixelSize: 12
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
                    color: "#141422"
                    radius: 8
                    border.color: Theme.grey
                    border.width: 1
                    opacity: bluetoothMenu.hasSelection ? 1 : 0
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
                                color: Theme.text
                                font.family: Theme.font
                                font.pixelSize: 12
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            Rectangle {
                                Layout.preferredWidth: 64
                                Layout.preferredHeight: 28
                                radius: 5
                                color: sideCloseMouse.containsMouse ? "#3a1f27" : "#2b1720"
                                border.color: Theme.grey
                                border.width: 1

                                RowLayout {
                                    anchors.centerIn: parent
                                    spacing: 4

                                    Text {
                                        text: "󰅖"
                                        color: Theme.text
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "Hide"
                                        color: Theme.text
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
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
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 14
                            font.bold: true
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "MAC: " + bluetoothMenu.selectedMac
                            color: Theme.hover
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 5

                            Text {
                                text: "Connected: " + (bluetoothMenu.selectedConnected || "no")
                                color: bluetoothMenu.selectedConnected === "yes" ? Theme.green : Theme.hover
                                font.family: Theme.font
                                font.pixelSize: 11
                            }

                            Text {
                                text: "•"
                                color: Theme.grey
                                font.family: Theme.font
                                font.pixelSize: 11
                            }

                            Text {
                                text: "Trusted: " + (bluetoothMenu.selectedTrusted || "no")
                                color: Theme.hover
                                font.family: Theme.font
                                font.pixelSize: 11
                            }

                            Text {
                                text: "•"
                                color: Theme.grey
                                font.family: Theme.font
                                font.pixelSize: 11
                            }

                            Text {
                                text: "Paired: " + (bluetoothMenu.selectedPaired || "no")
                                color: Theme.hover
                                font.family: Theme.font
                                font.pixelSize: 11
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Theme.grey
                            opacity: 0.8
                        }

                        Rectangle {
                            property bool isEnabled: bluetoothMenu.bluetoothEnabled && bluetoothMenu.selectedMac.length > 0 && bluetoothMenu.selectedPaired !== "yes"
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: !isEnabled ? "#23232d" : (pairMainMouse.containsMouse ? "#2e4060" : "#1e2e45")
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.6
                            visible: bluetoothMenu.selectedPaired !== "yes"

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰌹"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: "Pair"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: pairMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: bluetoothMenu.pairSelectedDevice()
                            }
                        }

                        Rectangle {
                            property bool isEnabled: bluetoothMenu.bluetoothEnabled && bluetoothMenu.selectedMac.length > 0 && bluetoothMenu.selectedConnected !== "yes"
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: !isEnabled ? "#23232d" : (connectMainMouse.containsMouse ? "#29503a" : "#1f3e2c")
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.6

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰖩"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: bluetoothMenu.selectedConnected === "yes" ? "Connected" : "Connect"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: connectMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: bluetoothMenu.connectSelectedDevice()
                            }
                        }

                        Rectangle {
                            property bool isEnabled: bluetoothMenu.selectedConnected === "yes"
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: !isEnabled ? "#23232d" : (disconnectMainMouse.containsMouse ? "#3a1f27" : "#2b1720")
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.6

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰖪"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: "Disconnect"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: disconnectMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: bluetoothMenu.disconnectCurrentDevice()
                            }
                        }

                        Rectangle {
                            property bool isEnabled: bluetoothMenu.selectedMac.length > 0
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: !isEnabled ? "#23232d" : (removeMainMouse.containsMouse ? "#402024" : "#2f171a")
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.6

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰆴"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: "Remove"
                                    color: Theme.text
                                    font.family: Theme.font
                                    font.pixelSize: 12
                                    font.bold: true
                                }
                            }

                            MouseArea {
                                id: removeMainMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: bluetoothMenu.forgetSelectedDevice()
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
                color: Theme.yellow
                font.family: Theme.font
                font.pixelSize: 11
                wrapMode: Text.Wrap
                visible: text.length > 0
            }

        }
    }
}
