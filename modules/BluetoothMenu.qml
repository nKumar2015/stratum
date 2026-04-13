import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import Quickshell.Io
import "../globals/DaemonRpc.js" as DaemonRpc

import "../globals"
import "../components"

ApplicationWindow {
    id: bluetoothMenu
    width: 700
    height: 560
    flags: Qt.Window | Qt.WindowStaysOnTopHint
    color: "transparent"

    function initializeWindow() {
        refreshAll();
    }

    function cleanupWindow() {
        actionWatchdogTimer.stop();
        powerStateSyncTimer.stop();
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
    property var actionFallbackCommand: []
    property bool actionFallbackTried: false
    property var scanFallbackCommand: []
    property bool scanFallbackTried: false
    property bool btStateFallbackTried: false
    property bool btListFallbackTried: false
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
    readonly property bool daemonPreferred: true

    function setStatusMessage(message, autoClear) {
        statusMessage = message;
        if (autoClear)
            statusClearTimer.restart();
        else
            statusClearTimer.stop();
    }

    function finishScanState() {
        scanning = false;
        BluetoothState.scanning = false;
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
            BluetoothState.connected = false;
            BluetoothState.scanning = false;
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
        if (daemonPreferred && DaemonRpc.canUse()) {
            btStateFallbackTried = false;
            btListFallbackTried = false;
            btStateProc.command = DaemonRpc.command("bluetooth.state", {});
            btListProc.command = DaemonRpc.command("bluetooth.list", {});
        }
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

        actionFallbackCommand = ["stratum-cli", "bluetooth", "pair", selectedMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.pair", { mac: selectedMac }, 15);
        else
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

        actionFallbackCommand = ["stratum-cli", "bluetooth", "connect", selectedMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.connect", { mac: selectedMac }, 15);
        else
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
        actionFallbackCommand = ["stratum-cli", "bluetooth", "disconnect", targetMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.disconnect", { mac: targetMac }, 15);
        else
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

        actionFallbackCommand = ["stratum-cli", "bluetooth", "forget", selectedMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.forget", { mac: selectedMac }, 15);
        else
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

        actionFallbackCommand = ["stratum-cli", "bluetooth", "trust", selectedMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.trust", { mac: selectedMac }, 15);
        else
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

        actionFallbackCommand = ["stratum-cli", "bluetooth", "untrust", selectedMac];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.untrust", { mac: selectedMac }, 15);
        else
            actionProc.command = ["stratum-cli", "bluetooth", "untrust", selectedMac];
        pendingAction = "untrust";
        pendingActionTarget = selectedName || selectedMac;
        setStatusMessage("Removing trust for " + pendingActionTarget + "...", false);
        actionWatchdogTimer.restart();
        actionProc.running = true;
    }

    function toggleBluetoothPower() {
        const target = bluetoothEnabled ? "off" : "on";
        actionFallbackCommand = ["stratum-cli", "bluetooth", "power", target];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("bluetooth.power", { target: target }, 15);
        else
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

        scanFallbackCommand = ["stratum-cli", "bluetooth", "scan"];
        scanFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            scanProc.command = DaemonRpc.command("bluetooth.scan", {}, 15);
        else
            scanProc.command = ["stratum-cli", "bluetooth", "scan"];
        scanning = true;
        BluetoothState.scanning = true;
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
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (bluetoothMenu.daemonPreferred && !bluetoothMenu.btStateFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        bluetoothMenu.btStateFallbackTried = true;
                        btStateProc.command = ["stratum-cli", "bluetooth", "state"];
                        btStateProc.running = true;
                        return;
                    }
                    bluetoothMenu.bluetoothEnabled = false;
                    BluetoothState.powered = false;
                    BluetoothState.connected = false;
                    BluetoothState.scanning = false;
                    bluetoothMenu.setStatusMessage(source && source.error ? String(source.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                bluetoothMenu.btStateFallbackTried = false;

                bluetoothMenu.bluetoothEnabled = String(source.powered || "no") === "yes";
                BluetoothState.powered = bluetoothMenu.bluetoothEnabled;

                if (bluetoothMenu.pendingPowerSyncTarget) {
                    const expectedOn = bluetoothMenu.pendingPowerSyncTarget === "on";
                    if (bluetoothMenu.bluetoothEnabled === expectedOn) {
                        bluetoothMenu.finishPowerStateSync(true);
                        return;
                    }
                }

                if (!bluetoothMenu.bluetoothEnabled) {
                    BluetoothState.connected = false;
                    BluetoothState.scanning = false;
                    const poweringOn = bluetoothMenu.pendingAction === "toggle" && bluetoothMenu.pendingActionTarget === "on";
                    if (!bluetoothMenu.pendingAutoScan && !bluetoothMenu.scanning && !poweringOn)
                        bluetoothMenu.clearDeviceState();
                }

                if (bluetoothMenu.visible && bluetoothMenu.pendingAutoScan) {
                    if (bluetoothMenu.bluetoothEnabled && !bluetoothMenu.scanning) {
                        bluetoothMenu.pendingAutoScan = false;
                        bluetoothMenu.autoScanRetryCount = 0;
                        autoScanRetryTimer.stop();
                        bluetoothMenu.startScan();
                    } else if (!bluetoothMenu.bluetoothEnabled && !bluetoothMenu.scanning) {
                        autoScanRetryTimer.restart();
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
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (bluetoothMenu.daemonPreferred && !bluetoothMenu.btListFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        bluetoothMenu.btListFallbackTried = true;
                        btListProc.command = ["stratum-cli", "bluetooth", "list"];
                        btListProc.running = true;
                        return;
                    }
                    bluetoothMenu.devices = [];
                    bluetoothMenu.setStatusMessage(source && source.error ? String(source.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                bluetoothMenu.btListFallbackTried = false;

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
                    BluetoothState.connected = true;
                } else {
                    bluetoothMenu.activeMac = "";
                    bluetoothMenu.activeName = "";
                    bluetoothMenu.activeTrusted = "";
                    bluetoothMenu.activePaired = "";
                    BluetoothState.connected = false;
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
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                const message = source ? String(source.output || source.error || "") : result;

                const {
                    pendingAction: action,
                    pendingActionTarget: target
                } = bluetoothMenu;
                const lowerMsg = (message || "").toLowerCase();

                if (!source?.ok) {
                    if (bluetoothMenu.daemonPreferred && !bluetoothMenu.actionFallbackTried && bluetoothMenu.actionFallbackCommand.length > 0) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        bluetoothMenu.actionFallbackTried = true;
                        actionProc.command = bluetoothMenu.actionFallbackCommand;
                        actionProc.running = true;
                        return;
                    }
                    const errorMsg = source?.error ? String(source.error) : bluetoothMenu.missingBluetoothctlMessage;
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

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }

                bluetoothMenu.pendingAction = "";
                bluetoothMenu.pendingActionTarget = "";
                bluetoothMenu.actionFallbackCommand = [];
                bluetoothMenu.actionFallbackTried = false;
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
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                const outputText = source ? String(source.output || "") : result;

                bluetoothMenu.mergeScanOutputDevices(outputText);
                bluetoothMenu.finishScanState();

                if (!source || source.ok !== true) {
                    if (bluetoothMenu.daemonPreferred && !bluetoothMenu.scanFallbackTried && bluetoothMenu.scanFallbackCommand.length > 0) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        bluetoothMenu.scanFallbackTried = true;
                        scanProc.command = bluetoothMenu.scanFallbackCommand;
                        scanProc.running = true;
                        return;
                    }
                    bluetoothMenu.setStatusMessage(source && source.error ? String(source.error) : bluetoothMenu.missingBluetoothctlMessage, true);
                } else if (outputText.length > 0 && outputText.toLowerCase().indexOf("failed") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan failed: " + outputText, true);
                } else if (outputText.length > 0 && outputText.toLowerCase().indexOf("error") !== -1) {
                    bluetoothMenu.setStatusMessage("Scan error: " + outputText, true);
                } else {
                    bluetoothMenu.setStatusMessage("Scan completed.", true);
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                bluetoothMenu.scanFallbackCommand = [];
                bluetoothMenu.scanFallbackTried = false;

                bluetoothMenu.refreshAll();
                bluetoothMenu.postScanSyncTimer.restart();
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: BluetoothState.showMenu = false
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
        border.color: Theme.palette.borderInactive
        border.width: 1
        clip: true

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

                CompactToggleButton {
                    id: btToggleButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 98
                    iconText: bluetoothMenu.pendingAction === "toggle" ? "󰔟" : (bluetoothMenu.bluetoothEnabled ? "󰂯" : "󰂲")
                    labelText: bluetoothMenu.pendingAction === "toggle" ? "Working" : (bluetoothMenu.bluetoothEnabled ? "On" : "Off")
                    enabled: bluetoothMenu.pendingAction !== "toggle"
                    onClicked: bluetoothMenu.toggleBluetoothPower()
                }

                CompactIconButton {
                    id: closeButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    buttonRadius: 6
                    iconText: "󰅖"
                    iconColor: Theme.palette.error
                    iconPixelSize: 13
                    borderColor: "transparent"
                    hoverBorderColor: "transparent"
                    onClicked: BluetoothState.showMenu = false
                }
            }

            ConnectionStatusCard {
                headlineText: bluetoothMenu.activeName ? "Connected to " + bluetoothMenu.activeName : "No active Bluetooth device"
                actionEnabled: bluetoothMenu.activeMac.length > 0
                actionIconText: "󰂲"
                actionIconPixelSize: 20
                actionTooltipText: "Disconnect"

                firstLabel: "MAC"
                firstValue: bluetoothMenu.activeMac ? bluetoothMenu.activeMac : "N/A"
                firstValueColor: bluetoothMenu.activeMac ? Theme.palette.secondary : Theme.palette.tertiary

                secondLabel: "Trusted"
                secondValue: bluetoothMenu.activeTrusted ? bluetoothMenu.activeTrusted : "N/A"
                secondValueColor: bluetoothMenu.activeTrusted ? Theme.palette.success : Theme.palette.error

                thirdLabel: "Paired"
                thirdValue: bluetoothMenu.activePaired ? bluetoothMenu.activePaired : "N/A"
                thirdValueColor: bluetoothMenu.activePaired ? Theme.palette.success : Theme.palette.error

                onActionClicked: bluetoothMenu.disconnectCurrentDevice()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                CompactIconButton {
                    id: refreshButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    buttonRadius: 6
                    iconText: "󰑐"
                    iconPixelSize: 13
                    borderColor: "transparent"
                    hoverBorderColor: "transparent"
                    onClicked: bluetoothMenu.refreshAll()

                    StyledIconToolTip {
                        visible: refreshButton.hovered
                        text: "Refresh"
                    }
                }

                CompactIconButton {
                    id: scanButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    buttonRadius: 6
                    iconText: ""
                    iconPixelSize: 13
                    enabled: bluetoothMenu.bluetoothEnabled
                    borderColor: "transparent"
                    hoverBorderColor: "transparent"
                    onClicked: bluetoothMenu.startScan()

                    StyledIconToolTip {
                        visible: scanButton.hovered
                        text: "Scan"
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

                                delegate: HoverListRow {
                                    id: deviceRow
                                    required property var modelData
                                    required property var index
                                    property bool shouldAnimateOnCreate: bluetoothMenu.animateRowsOnNextLoad

                                    width: parent.width
                                    rowHeight: 56
                                    rowRadius: 6
                                    showLabel: false
                                    isActive: bluetoothMenu.selectedMac === modelData.mac
                                    highlightOnHover: false
                                    backgroundColor: Theme.palette.bgWidget
                                    activeBackgroundColor: Theme.palette.bgHover
                                    borderColor: modelData.connected === "yes" ? Theme.palette.borderActive : Theme.palette.borderInactive
                                    activeBorderColor: modelData.connected === "yes" ? Theme.palette.borderActive : Theme.palette.borderInactive
                                    contentLeftMargin: 8
                                    contentRightMargin: 8
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

                                    RowLayout {
                                        anchors.fill: parent
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

                                            TripleStatusStrip {
                                                firstLabel: "MAC"
                                                firstValue: modelData.mac
                                                firstLabelColor: Theme.palette.tertiary
                                                firstValueColor: bluetoothMenu.selectedMac === modelData.mac ? Theme.palette.secondary : Theme.palette.tertiary

                                                secondLabel: "State"
                                                secondValue: modelData.connected === "yes" ? "Connected" : "Disconnected"
                                                secondLabelColor: Theme.palette.tertiary
                                                secondValueColor: modelData.connected === "yes" ? Theme.palette.success : Theme.palette.error

                                                thirdLabel: "Trust"
                                                thirdValue: modelData.trusted === "yes" ? "Trusted" : "Untrusted"
                                                thirdLabelColor: Theme.palette.tertiary
                                                thirdValueColor: modelData.trusted === "yes" ? Theme.palette.success : Theme.palette.error

                                                labelPixelSize: 10
                                                valuePixelSize: 10
                                                dotPixelSize: 10
                                                valueBold: false
                                                thirdValueFillWidth: false
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

                    SelectedDevicePanel {
                        anchors.fill: parent
                        panelTitle: "Selected Device"
                        selectedName: bluetoothMenu.selectedName
                        selectedMac: bluetoothMenu.selectedMac
                        selectedConnected: bluetoothMenu.selectedConnected
                        selectedTrusted: bluetoothMenu.selectedTrusted
                        selectedPaired: bluetoothMenu.selectedPaired
                        hasSelection: bluetoothMenu.selectedMac.length > 0
                        actionInProgress: bluetoothMenu.pendingAction.length !== 0

                        onCloseClicked: bluetoothMenu.clearSelection()
                        onPairClicked: bluetoothMenu.selectedPaired ? bluetoothMenu.forgetSelectedDevice() : bluetoothMenu.pairSelectedDevice()
                        onConnectClicked: bluetoothMenu.selectedConnected ? bluetoothMenu.disconnectCurrentDevice() : bluetoothMenu.connectSelectedDevice()
                        onTrustClicked: bluetoothMenu.selectedTrusted ? bluetoothMenu.untrustSelectedDevice() : bluetoothMenu.trustSelectedDevice()
                    }
                }
            }

            StatusMessageFooter {
                messageText: bluetoothMenu.statusMessage
            }
        }
    }
}
