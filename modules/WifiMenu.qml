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
    id: wifiMenu
    width: 700
    height: 560
    flags: Qt.Window | Qt.WindowStaysOnTopHint
    color: "transparent"

    function initializeWindow() {
        refreshAll();
        refreshTimer.start();
    }

    function cleanupWindow() {
        refreshTimer.stop();
    }

    

    property bool wifiEnabled: true
    property string activeDevice: ""
    property string activeSsid: ""
    property string activeState: ""
    property int activeSignal: -1
    property string activeSecurity: ""
    property string activeIp: ""
    property string activeGateway: ""
    property string selectedSsid: ""
    property string selectedSecurity: ""
    property int selectedSignal: -1
    property string selectedInUse: ""
    property bool hasSelection: selectedSsid.length > 0
    property string statusMessage: ""
    property var networks: []
    property int hiddenDuplicateCount: 0
    property bool listLoading: false
    property bool animateRowsOnNextLoad: true
    property string pendingAction: ""
    property string pendingActionTarget: ""
    property string autoHideOnConnectSsid: ""
    property var knownWifiSsids: ({})
    property bool requirePasswordRetry: false
    property bool pendingConnectWasKnown: false
    property bool pendingConnectWasSecure: false
    property var actionFallbackCommand: []
    property bool wifiStateFallbackTried: false
    property bool deviceStatusFallbackTried: false
    property bool knownConnectionsFallbackTried: false
    property bool wifiListFallbackTried: false
    property bool activeInfoFallbackTried: false
    property bool actionFallbackTried: false
    readonly property string missingNmcliMessage: "nmcli is required for Wi-Fi controls."
    readonly property int signalStrongThreshold: 75
    readonly property int signalGoodThreshold: 50
    readonly property int signalFairThreshold: 25
    readonly property int refreshIntervalMs: 12000
    readonly property bool daemonPreferred: true

    function isSecureNetwork(security) {
        if (!security)
            return false;
        const value = security.trim().toLowerCase();
        return value !== "" && value !== "--" && value !== "none";
    }

    function splitNmcliFields(line, expectedFields) {
        const fields = [];
        let current = "";
        let escaped = false;

        for (let i = 0; i < line.length; i++) {
            const ch = line[i];
            if (escaped) {
                current += ch;
                escaped = false;
                continue;
            }

            if (ch === "\\") {
                escaped = true;
                continue;
            }

            if (ch === ":" && fields.length < expectedFields - 1) {
                fields.push(current);
                current = "";
                continue;
            }

            current += ch;
        }

        fields.push(current);
        return fields;
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

    function signalLevel(signal) {
        if (signal >= signalStrongThreshold)
            return 4;
        if (signal >= signalGoodThreshold)
            return 3;
        if (signal >= signalFairThreshold)
            return 2;
        if (signal > 0)
            return 1;
        return 0;
    }

    function signalBars(signal) {
        const level = signalLevel(signal);
        let bars = "";
        for (let i = 0; i < 4; i++)
            bars += i < level ? "▮" : "▯";
        return bars;
    }

    function isKnownNetwork(ssid) {
        return !!knownWifiSsids[ssid];
    }

    function shouldShowPasswordField() {
        return hasSelection && isSecureNetwork(selectedSecurity) && (!isKnownNetwork(selectedSsid) || requirePasswordRetry);
    }

    function refreshAll() {
        listLoading = true;
        if (daemonPreferred && DaemonRpc.canUse()) {
            wifiStateFallbackTried = false;
            deviceStatusFallbackTried = false;
            knownConnectionsFallbackTried = false;
            wifiListFallbackTried = false;
            wifiStateProc.command = DaemonRpc.command("wifi.state", {});
            deviceStatusProc.command = DaemonRpc.command("wifi.device_status", {});
            wifiListProc.command = DaemonRpc.command("wifi.list", {});
            knownConnectionsProc.command = DaemonRpc.command("wifi.known_connections", {});
        }
        wifiStateProc.running = true;
        deviceStatusProc.running = true;
        wifiListProc.running = true;
        knownConnectionsProc.running = true;
    }

    function clearSelection() {
        selectedSsid = "";
        selectedSecurity = "";
        selectedSignal = -1;
        selectedInUse = "";
        requirePasswordRetry = false;
        selectedWifiPanel.passwordText = "";
    }

    function refreshActiveInfo() {
        if (!activeDevice) {
            activeIp = "";
            activeGateway = "";
            return;
        }

        activeInfoFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            activeInfoProc.command = DaemonRpc.command("wifi.active_info", { device: activeDevice });
        else
            activeInfoProc.command = ["stratum-cli", "wifi", "active-info", activeDevice];
        activeInfoProc.running = true;
    }

    function connectSelectedNetwork() {
        if (!selectedSsid)
            return;

        const showPassword = shouldShowPasswordField();
        const trimmedPassword = selectedWifiPanel.passwordText.trim();
        if (showPassword && trimmedPassword.length === 0) {
            statusMessage = "Enter password to connect.";
            return;
        }

        actionFallbackCommand = ["stratum-cli", "wifi", "connect", selectedSsid];
        if (showPassword)
            actionFallbackCommand.push(trimmedPassword);
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("wifi.connect", showPassword ? {
                                                            ssid: selectedSsid,
                                                            password: trimmedPassword
                                                        } : {
                                                            ssid: selectedSsid
                                                        }, 15);
        else {
            const cmd = ["stratum-cli", "wifi", "connect", selectedSsid];
            if (showPassword)
                cmd.push(trimmedPassword);
            actionProc.command = cmd;
        }
        pendingAction = "connect";
        pendingActionTarget = selectedSsid;
        pendingConnectWasKnown = isKnownNetwork(selectedSsid);
        pendingConnectWasSecure = isSecureNetwork(selectedSecurity);
        autoHideOnConnectSsid = selectedSsid;
        statusMessage = "Connecting to " + selectedSsid + "...";
        actionProc.running = true;
    }

    function disconnectCurrentNetwork() {
        if (!activeDevice)
            return;

        actionFallbackCommand = ["stratum-cli", "wifi", "disconnect", activeDevice];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("wifi.disconnect", { device: activeDevice }, 15);
        else
            actionProc.command = ["stratum-cli", "wifi", "disconnect", activeDevice];
        pendingAction = "disconnect";
        pendingActionTarget = activeSsid;
        statusMessage = "Disconnecting " + activeSsid + "...";
        actionProc.running = true;
    }

    function forgetCurrentNetwork() {
        if (!activeSsid)
            return;

        pendingAction = "forget";
        pendingActionTarget = activeSsid;
        actionFallbackCommand = ["stratum-cli", "wifi", "forget", activeSsid];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("wifi.forget", { ssid: activeSsid }, 15);
        else
            actionProc.command = ["stratum-cli", "wifi", "forget", activeSsid];
        statusMessage = "Forgetting " + activeSsid + "...";
        actionProc.running = true;
    }

    function toggleWifiRadio() {
        const target = wifiEnabled ? "off" : "on";
        actionFallbackCommand = ["stratum-cli", "wifi", "toggle", target];
        actionFallbackTried = false;
        if (daemonPreferred && DaemonRpc.canUse())
            actionProc.command = DaemonRpc.command("wifi.toggle", { target: target }, 15);
        else
            actionProc.command = ["stratum-cli", "wifi", "toggle", target];
        pendingAction = "toggle";
        pendingActionTarget = target;
        statusMessage = wifiEnabled ? "Turning Wi-Fi off..." : "Turning Wi-Fi on...";
        actionProc.running = true;
    }

    Process {
        id: wifiStateProc
        command: ["stratum-cli", "wifi", "state"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.wifiStateFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.wifiStateFallbackTried = true;
                        wifiStateProc.command = ["stratum-cli", "wifi", "state"];
                        wifiStateProc.running = true;
                        return;
                    }
                    wifiMenu.statusMessage = wifiMenu.missingNmcliMessage;
                    wifiMenu.wifiEnabled = false;
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                wifiMenu.wifiStateFallbackTried = false;

                const state = String(source.state || "").toLowerCase();
                wifiMenu.wifiEnabled = state.indexOf("enabled") !== -1;
            }
        }
    }

    Process {
        id: deviceStatusProc
        command: ["stratum-cli", "wifi", "device-status"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.deviceStatusFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.deviceStatusFallbackTried = true;
                        deviceStatusProc.command = ["stratum-cli", "wifi", "device-status"];
                        deviceStatusProc.running = true;
                        return;
                    }
                    wifiMenu.statusMessage = wifiMenu.missingNmcliMessage;
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                wifiMenu.deviceStatusFallbackTried = false;

                const rows = Array.isArray(source.devices) ? source.devices : [];
                let currentDevice = "";
                let currentSsid = "";
                let currentState = "";

                for (let i = 0; i < rows.length; i++) {
                    const row = rows[i] || {};
                    const device = String(row.device || "").trim();
                    const type = String(row.type || "").trim().toLowerCase();
                    const state = String(row.state || "").trim().toLowerCase();
                    const conn = String(row.connection || "").trim();

                    if (type === "wifi" && state.indexOf("connected") !== -1) {
                        currentDevice = device;
                        currentSsid = conn;
                        currentState = String(row.state || "").trim();
                        break;
                    }
                }

                wifiMenu.activeDevice = currentDevice;
                wifiMenu.activeSsid = currentSsid;
                wifiMenu.activeState = currentState;

                if (wifiMenu.activeDevice)
                    wifiMenu.refreshActiveInfo();
                else {
                    wifiMenu.activeIp = "";
                    wifiMenu.activeGateway = "";
                    wifiMenu.activeSignal = -1;
                    wifiMenu.activeSecurity = "";
                }
            }
        }
    }

    Process {
        id: knownConnectionsProc
        command: ["stratum-cli", "wifi", "known-connections"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.knownConnectionsFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.knownConnectionsFallbackTried = true;
                        knownConnectionsProc.command = ["stratum-cli", "wifi", "known-connections"];
                        knownConnectionsProc.running = true;
                        return;
                    }
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                wifiMenu.knownConnectionsFallbackTried = false;

                const rows = Array.isArray(source.connections) ? source.connections : [];
                const known = {};

                for (let i = 0; i < rows.length; i++) {
                    const row = rows[i] || {};
                    const name = String(row.name || "").trim();
                    const type = String(row.type || "").trim().toLowerCase();
                    if (!name)
                        continue;

                    if (type.indexOf("wifi") !== -1 || type.indexOf("wireless") !== -1)
                        known[name] = true;
                }

                wifiMenu.knownWifiSsids = known;
            }
        }
    }

    Process {
        id: wifiListProc
        command: ["stratum-cli", "wifi", "list"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                wifiMenu.listLoading = false;
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.wifiListFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.wifiListFallbackTried = true;
                        wifiListProc.command = ["stratum-cli", "wifi", "list"];
                        wifiListProc.running = true;
                        return;
                    }
                    wifiMenu.statusMessage = wifiMenu.missingNmcliMessage;
                    wifiMenu.networks = [];
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                wifiMenu.wifiListFallbackTried = false;

                const rows = Array.isArray(source.networks) ? source.networks : [];
                const dedupBySsid = {};
                let candidateCount = 0;

                for (let i = 0; i < rows.length; i++) {
                    const row = rows[i] || {};
                    const inUse = String(row.in_use || "").trim();
                    const ssid = String(row.ssid || "").trim();
                    if (!ssid)
                        continue;

                    const signalValue = Number(row.signal || 0);

                    const candidate = {
                        inUse: inUse,
                        ssid: ssid,
                        signal: isNaN(signalValue) ? 0 : Math.round(signalValue),
                        security: String(row.security || "").trim()
                    };
                    candidateCount++;

                    const existing = dedupBySsid[ssid];
                    if (!existing) {
                        dedupBySsid[ssid] = candidate;
                        continue;
                    }

                    // Dedup priority: currently connected entry first, then strongest signal.
                    const candidateConnected = candidate.inUse === "*";
                    const existingConnected = existing.inUse === "*";

                    if ((candidateConnected && !existingConnected) || (!existingConnected && candidate.signal > existing.signal)) {
                        dedupBySsid[ssid] = candidate;
                    } else if (!existing.security && candidate.security) {
                        existing.security = candidate.security;
                    }
                }

                const parsed = Object.values(dedupBySsid);
                wifiMenu.hiddenDuplicateCount = Math.max(0, candidateCount - parsed.length);
                parsed.sort(function (a, b) {
                    const aConnected = a.inUse === "*";
                    const bConnected = b.inUse === "*";

                    if (aConnected !== bConnected)
                        return aConnected ? -1 : 1;

                    if (a.signal !== b.signal)
                        return b.signal - a.signal;

                    return a.ssid.localeCompare(b.ssid);
                });

                wifiMenu.networks = parsed;
                if (wifiMenu.animateRowsOnNextLoad)
                    wifiMenu.animateRowsOnNextLoad = false;

                let selectedFound = false;
                for (let i = 0; i < parsed.length; i++) {
                    if (parsed[i].ssid === wifiMenu.selectedSsid) {
                        wifiMenu.selectedSecurity = parsed[i].security;
                        wifiMenu.selectedSignal = parsed[i].signal;
                        wifiMenu.selectedInUse = parsed[i].inUse;

                        if (parsed[i].inUse === "*" && wifiMenu.autoHideOnConnectSsid === parsed[i].ssid) {
                            wifiMenu.statusMessage = "Connected to " + parsed[i].ssid + ".";
                            wifiMenu.autoHideOnConnectSsid = "";
                            wifiMenu.clearSelection();
                            selectedFound = false;
                            break;
                        }

                        selectedFound = true;
                        break;
                    }
                }

                if (!selectedFound) {
                    wifiMenu.clearSelection();
                }

                for (let i = 0; i < parsed.length; i++) {
                    if (parsed[i].inUse === "*") {
                        wifiMenu.activeSignal = parsed[i].signal;
                        wifiMenu.activeSecurity = parsed[i].security;
                        break;
                    }
                }
            }
        }
    }

    Process {
        id: activeInfoProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;
                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.activeInfoFallbackTried) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.activeInfoFallbackTried = true;
                        activeInfoProc.command = ["stratum-cli", "wifi", "active-info", wifiMenu.activeDevice];
                        activeInfoProc.running = true;
                        return;
                    }
                    wifiMenu.statusMessage = wifiMenu.missingNmcliMessage;
                    return;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }
                wifiMenu.activeInfoFallbackTried = false;

                wifiMenu.activeIp = String(source.ip4_address || "");
                wifiMenu.activeGateway = String(source.ip4_gateway || "");
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);
                const source = (payload && payload.jsonrpc === "2.0" && payload.result) ? payload.result : payload;

                if (!source || source.ok !== true) {
                    if (wifiMenu.daemonPreferred && !wifiMenu.actionFallbackTried && wifiMenu.actionFallbackCommand.length > 0) {
                        DaemonRpc.recordFailure();
                        AudioState.daemonAvailable = false;
                        wifiMenu.actionFallbackTried = true;
                        actionProc.command = wifiMenu.actionFallbackCommand;
                        actionProc.running = true;
                        return;
                    }
                    if (wifiMenu.pendingAction === "connect" && wifiMenu.pendingConnectWasKnown && wifiMenu.pendingConnectWasSecure && !wifiMenu.requirePasswordRetry) {
                        wifiMenu.requirePasswordRetry = true;
                        wifiMenu.statusMessage = "Saved credentials failed. Enter password and retry.";
                    } else {
                        wifiMenu.statusMessage = source && source.error ? String(source.error) : (payload && payload.error ? String(payload.error) : "Action failed.");
                    }
                    wifiMenu.autoHideOnConnectSsid = "";
                } else if (String(source.message || "").toLowerCase().indexOf("error") !== -1) {
                    if (wifiMenu.pendingAction === "connect" && wifiMenu.pendingConnectWasKnown && wifiMenu.pendingConnectWasSecure && !wifiMenu.requirePasswordRetry) {
                        wifiMenu.requirePasswordRetry = true;
                        wifiMenu.statusMessage = "Saved credentials failed. Enter password and retry.";
                    } else {
                        wifiMenu.statusMessage = String(source.message || "Action failed.");
                    }
                    wifiMenu.autoHideOnConnectSsid = "";
                } else if (wifiMenu.pendingAction === "forget") {
                    wifiMenu.statusMessage = "Forgot network " + wifiMenu.pendingActionTarget + ".";
                } else {
                    wifiMenu.statusMessage = "Action completed.";
                    if (wifiMenu.pendingAction === "connect")
                        wifiMenu.requirePasswordRetry = false;
                }

                if (payload && payload.jsonrpc === "2.0") {
                    DaemonRpc.recordSuccess();
                    AudioState.daemonAvailable = true;
                }

                wifiMenu.pendingAction = "";
                wifiMenu.pendingActionTarget = "";
                wifiMenu.pendingConnectWasKnown = false;
                wifiMenu.pendingConnectWasSecure = false;
                wifiMenu.actionFallbackCommand = [];
                wifiMenu.actionFallbackTried = false;

                if (!wifiMenu.shouldShowPasswordField())
                    selectedWifiPanel.passwordText = "";
                wifiMenu.refreshAll();
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: WifiState.showMenu = false
    }

    onVisibleChanged: {
        if (visible) {
            animateRowsOnNextLoad = true;
            refreshAll();
            refreshTimer.running = true;
        } else {
            refreshTimer.running = false;
        }
    }

    Timer {
        id: refreshTimer
        interval: wifiMenu.refreshIntervalMs
        repeat: true
        running: false
        onTriggered: wifiMenu.refreshAll()
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
                    text: "Wi-Fi"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 18
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                CompactToggleButton {
                    id: wifiToggleButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 98
                    iconText: wifiMenu.wifiEnabled ? "󰤨" : "󰤮"
                    labelText: wifiMenu.wifiEnabled ? "On" : "Off"
                    onClicked: wifiMenu.toggleWifiRadio()
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
                    onClicked: WifiState.showMenu = false
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 170
                color: Theme.palette.bgWidget
                radius: 8

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: 0

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            text: wifiMenu.activeSsid ? "Connected to " + wifiMenu.activeSsid : "Not connected"
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 13
                            font.bold: true
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        CompactIconButton {
                            id: disconnectButton
                            Layout.preferredHeight: 30
                            Layout.preferredWidth: 38
                            buttonRadius: 6
                            iconText: "󰖪"
                            iconPixelSize: 20
                            enabled: wifiMenu.activeDevice.length > 0
                            disabledBackgroundColor: Theme.palette.bgDark
                            borderColor: "transparent"
                            hoverBorderColor: "transparent"
                            onClicked: wifiMenu.disconnectCurrentNetwork()

                            StyledIconToolTip {
                                visible: disconnectButton.hovered
                                text: "Disconnect"
                            }
                        }

                        CompactIconButton {
                            id: forgetButton
                            Layout.preferredHeight: 30
                            Layout.preferredWidth: 38
                            buttonRadius: 6
                            iconText: "󰆴"
                            iconPixelSize: 14
                            enabled: wifiMenu.activeSsid.length > 0
                            disabledBackgroundColor: Theme.palette.bgDark
                            borderColor: "transparent"
                            hoverBorderColor: "transparent"
                            onClicked: wifiMenu.forgetCurrentNetwork()

                            StyledIconToolTip {
                                visible: forgetButton.hovered
                                text: "Forget"
                            }
                        }
                    }

                    TripleStatusStrip {
                        firstLabel: "State"
                        firstValue: wifiMenu.activeState ? wifiMenu.activeState : (wifiMenu.wifiEnabled ? "idle" : "wifi disabled")
                        firstLabelColor: Theme.palette.secondary
                        firstValueColor: Theme.palette.tertiary

                        secondLabel: "Signal"
                        secondValue: wifiMenu.activeSignal >= 0 ? wifiMenu.signalBars(wifiMenu.activeSignal) : "N/A"
                        secondLabelColor: Theme.palette.secondary
                        secondValueColor: wifiMenu.activeSignal >= 50 ? Theme.palette.success : Theme.palette.warning

                        thirdLabel: "Security"
                        thirdValue: wifiMenu.activeSecurity ? wifiMenu.activeSecurity : "N/A"
                        thirdLabelColor: Theme.palette.secondary
                        thirdValueColor: Theme.palette.tertiary

                        labelPixelSize: 12
                        valuePixelSize: 12
                        dotPixelSize: 11
                        valueBold: false
                        thirdValueFillWidth: true
                    }

                    StatusRow {
                        Layout.fillWidth: true
                        labelText: "IP"
                        valueText: wifiMenu.activeIp ? wifiMenu.activeIp : "N/A"
                        labelColor: Theme.palette.secondary
                        valueColor: Theme.palette.tertiary
                        labelPixelSize: 12
                        valuePixelSize: 12
                        valueBold: false
                    }

                    StatusRow {
                        Layout.fillWidth: true
                        labelText: "Gateway"
                        valueText: wifiMenu.activeGateway ? wifiMenu.activeGateway : "N/A"
                        labelColor: Theme.palette.secondary
                        valueColor: Theme.palette.tertiary
                        labelPixelSize: 12
                        valuePixelSize: 12
                        valueBold: false
                    }
                }
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
                    onClicked: wifiMenu.refreshAll()

                    StyledIconToolTip {
                        visible: refreshButton.hovered
                        text: "Refresh"
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: wifiMenu.networks.length + " networks" + (wifiMenu.hiddenDuplicateCount > 0 ? "  •  " + wifiMenu.hiddenDuplicateCount + " de-duplicated" : "")
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                }

                RowLayout {
                    spacing: 6
                    visible: wifiMenu.listLoading

                    Rectangle {
                        implicitWidth: 8
                        implicitHeight: 8
                        radius: 4
                        color: Theme.palette.success

                        SequentialAnimation on opacity {
                            running: wifiMenu.listLoading
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
                        text: "Scanning"
                        color: Theme.palette.success
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: wifiMenu.hasSelection ? 10 : 0

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.palette.bgMain
                    radius: 8

                    ScrollView {
                        id: wifiNetworkScroll
                        anchors.fill: parent
                        anchors.margins: 8
                        clip: true
                        contentWidth: availableWidth

                        Column {
                            width: wifiNetworkScroll.availableWidth
                            spacing: 6

                            Repeater {
                                model: wifiMenu.networks

                                delegate: HoverListRow {
                                    id: networkRow
                                    required property int index
                                    required property var modelData
                                    property bool shouldAnimateOnCreate: wifiMenu.animateRowsOnNextLoad

                                    width: parent.width
                                    rowHeight: 56
                                    rowRadius: 6
                                    showLabel: false
                                    isActive: wifiMenu.selectedSsid === modelData.ssid
                                    highlightOnHover: false
                                    backgroundColor: Theme.palette.bgWidget
                                    activeBackgroundColor: Theme.palette.bgHover
                                    borderColor: modelData.inUse === "*" ? Theme.palette.borderActive : Theme.palette.borderInactive
                                    activeBorderColor: modelData.inUse === "*" ? Theme.palette.borderActive : Theme.palette.borderInactive
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
                                            target: networkRow
                                            property: "opacity"
                                            from: 0
                                            to: 1
                                            duration: 170
                                            easing.type: Easing.OutCubic
                                        }
                                    }

                                    Component.onCompleted: {
                                        if (networkRow.shouldAnimateOnCreate) {
                                            networkRow.opacity = 0;
                                            rowFadeIn.restart();
                                        } else {
                                            networkRow.opacity = 1;
                                        }
                                    }

                                    onModelDataChanged: {
                                        // Ensure reused delegates never stay hidden after list refreshes.
                                        if (!rowFadeIn.running)
                                            networkRow.opacity = 1;
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
                                        if (wifiMenu.selectedSsid === modelData.ssid) {
                                            wifiMenu.clearSelection();
                                        } else {
                                            wifiMenu.selectedSsid = modelData.ssid;
                                            wifiMenu.selectedSecurity = modelData.security;
                                            wifiMenu.selectedSignal = modelData.signal;
                                            wifiMenu.selectedInUse = modelData.inUse;
                                            wifiMenu.requirePasswordRetry = false;
                                            selectedWifiPanel.passwordText = "";
                                        }
                                    }

                                    RowLayout {
                                        anchors.fill: parent
                                        spacing: 8

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Text {
                                                text: modelData.ssid
                                                color: Theme.palette.textMain
                                                font.family: Theme.palette.font
                                                font.pixelSize: 12
                                                elide: Text.ElideRight
                                            }

                                            TripleStatusStrip {
                                                firstLabel: "Signal"
                                                firstValue: wifiMenu.signalBars(modelData.signal)
                                                firstLabelColor: Theme.palette.secondary
                                                firstValueColor: modelData.signal >= 50 ? Theme.palette.success : Theme.palette.warning

                                                secondLabel: "Security"
                                                secondValue: modelData.security ? modelData.security : "Open"
                                                secondLabelColor: Theme.palette.secondary
                                                secondValueColor: Theme.palette.tertiary

                                                thirdLabel: "State"
                                                thirdValue: modelData.inUse === "*" ? "Connected" : "Disconnected"
                                                thirdLabelColor: Theme.palette.secondary
                                                thirdValueColor: modelData.inUse === "*" ? Theme.palette.success : Theme.palette.error

                                                labelPixelSize: 12
                                                valuePixelSize: 10
                                                dotPixelSize: 10
                                                valueBold: false
                                                thirdValueFillWidth: false
                                            }
                                        }

                                        Item {
                                            Layout.fillWidth: true
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
                    Layout.preferredWidth: wifiMenu.hasSelection ? 250 : 0
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

                    SelectedNetworkPanel {
                        id: selectedWifiPanel
                        anchors.fill: parent
                        panelTitle: "Selected Network"
                        selectedSsid: wifiMenu.selectedSsid
                        selectedInUse: wifiMenu.selectedInUse
                        selectedSignalDisplay: wifiMenu.selectedSignal >= 0 ? wifiMenu.signalBars(wifiMenu.selectedSignal) : "N/A"
                        selectedSecurity: wifiMenu.selectedSecurity
                        hasSelection: wifiMenu.selectedSsid.length > 0
                        wifiEnabled: wifiMenu.wifiEnabled
                        showPasswordField: wifiMenu.shouldShowPasswordField()
                        canDisconnect: wifiMenu.activeDevice.length > 0

                        onCloseClicked: wifiMenu.clearSelection()
                        onConnectClicked: wifiMenu.selectedInUse === "*" ? wifiMenu.disconnectCurrentNetwork() : wifiMenu.connectSelectedNetwork()
                    }
                }
            }

            StatusMessageFooter {
                messageText: wifiMenu.statusMessage
            }
        }
    }
}
