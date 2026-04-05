import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import Quickshell
import Quickshell.Io

import "../theme"
import "../globals"

Window {
    id: wifiMenu

    title: "Wi-Fi"
    flags: Qt.Window

    visible: GlobalState.showWifiSettings

    width: 700
    height: 560
    color: "transparent"

    onClosing: {
        close.accepted = false;
        GlobalState.showWifiSettings = false;
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
        if (signal >= 75)
            return 4;
        if (signal >= 50)
            return 3;
        if (signal >= 25)
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
        passwordInput.text = "";
    }

    function refreshActiveInfo() {
        if (!activeDevice) {
            activeIp = "";
            activeGateway = "";
            return;
        }

        activeInfoProc.command = ["stratum-cli", "wifi", "active-info", activeDevice];
        activeInfoProc.running = true;
    }

    function connectSelectedNetwork() {
        if (!selectedSsid)
            return;

        const showPassword = shouldShowPasswordField();
        const trimmedPassword = passwordInput.text.trim();
        if (showPassword && trimmedPassword.length === 0) {
            statusMessage = "Enter password to connect.";
            return;
        }

        const cmd = ["stratum-cli", "wifi", "connect", selectedSsid];
        if (showPassword)
            cmd.push(trimmedPassword);
        actionProc.command = cmd;
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
        actionProc.command = ["stratum-cli", "wifi", "forget", activeSsid];
        statusMessage = "Forgetting " + activeSsid + "...";
        actionProc.running = true;
    }

    function toggleWifiRadio() {
        const target = wifiEnabled ? "off" : "on";
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
                if (!payload || payload.ok !== true) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    wifiMenu.wifiEnabled = false;
                    return;
                }

                const state = String(payload.state || "").toLowerCase();
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
                if (!payload || payload.ok !== true) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    return;
                }

                const rows = Array.isArray(payload.devices) ? payload.devices : [];
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
                if (!payload || payload.ok !== true)
                    return;

                const rows = Array.isArray(payload.connections) ? payload.connections : [];
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
                if (!payload || payload.ok !== true) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    wifiMenu.networks = [];
                    return;
                }

                const rows = Array.isArray(payload.networks) ? payload.networks : [];
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
                parsed.sort(function(a, b) {
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
                if (!payload || payload.ok !== true) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    return;
                }

                wifiMenu.activeIp = String(payload.ip4_address || "");
                wifiMenu.activeGateway = String(payload.ip4_gateway || "");
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = wifiMenu.parseCliJson(result);

                if (!payload || payload.ok !== true) {
                    if (wifiMenu.pendingAction === "connect" && wifiMenu.pendingConnectWasKnown && wifiMenu.pendingConnectWasSecure && !wifiMenu.requirePasswordRetry) {
                        wifiMenu.requirePasswordRetry = true;
                        wifiMenu.statusMessage = "Saved credentials failed. Enter password and retry.";
                    } else {
                        wifiMenu.statusMessage = payload && payload.error ? String(payload.error) : "Action failed.";
                    }
                    wifiMenu.autoHideOnConnectSsid = "";
                } else if (String(payload.message || "").toLowerCase().indexOf("error") !== -1) {
                    if (wifiMenu.pendingAction === "connect" && wifiMenu.pendingConnectWasKnown && wifiMenu.pendingConnectWasSecure && !wifiMenu.requirePasswordRetry) {
                        wifiMenu.requirePasswordRetry = true;
                        wifiMenu.statusMessage = "Saved credentials failed. Enter password and retry.";
                    } else {
                        wifiMenu.statusMessage = String(payload.message || "Action failed.");
                    }
                    wifiMenu.autoHideOnConnectSsid = "";
                } else if (wifiMenu.pendingAction === "forget") {
                    wifiMenu.statusMessage = "Forgot network " + wifiMenu.pendingActionTarget + ".";
                } else {
                    wifiMenu.statusMessage = "Action completed.";
                    if (wifiMenu.pendingAction === "connect")
                        wifiMenu.requirePasswordRetry = false;
                }

                wifiMenu.pendingAction = "";
                wifiMenu.pendingActionTarget = "";
                wifiMenu.pendingConnectWasKnown = false;
                wifiMenu.pendingConnectWasSecure = false;

                if (!wifiMenu.shouldShowPasswordField())
                    passwordInput.text = "";
                wifiMenu.refreshAll();
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: GlobalState.showWifiSettings = false
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
        interval: 12000
        repeat: true
        running: false
        onTriggered: wifiMenu.refreshAll()
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
                    text: "Wi-Fi"
                    color: Theme.on_Surface
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
                    color: wifiToggleMouse.containsMouse ? "#263244" : "#1b2333"
                    border.color: Theme.outlineVariant
                    border.width: 1

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: wifiMenu.wifiEnabled ? "󰤨" : "󰤮"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 13
                        }

                        Text {
                            text: wifiMenu.wifiEnabled ? "On" : "Off"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 12
                            font.bold: true
                        }
                    }

                    MouseArea {
                        id: wifiToggleMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: wifiMenu.toggleWifiRadio()
                    }
                }

                Rectangle {
                    id: closeButton
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 38
                    radius: 6
                    color: closeMouse.containsMouse ? "#3a1f27" : "#2b1720"
                    border.color: Theme.outlineVariant
                    border.width: 1

                    StyledIconToolTip {
                        visible: closeMouse.containsMouse
                        text: "Close"
                    }

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: "󰅖"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 13
                        }
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: GlobalState.showWifiSettings = false
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 170
                color: Theme.background
                radius: 8
                border.color: Theme.outlineVariant
                border.width: 1

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: 6

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8

                        Text {
                            text: wifiMenu.activeSsid ? "Connected to " + wifiMenu.activeSsid : "Not connected"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 13
                            font.bold: true
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Rectangle {
                            id: disconnectButton
                            property bool isEnabled: wifiMenu.activeDevice.length > 0
                            Layout.preferredHeight: 30
                            Layout.preferredWidth: 38
                            radius: 6
                            color: !isEnabled ? "#23232d" : (topDisconnectMouse.containsMouse ? "#3a1f27" : "#2b1720")
                            border.color: Theme.outlineVariant
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.5

                            StyledIconToolTip {
                                visible: topDisconnectMouse.containsMouse
                                text: "Disconnect"
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰖪"
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 12
                            }

                            MouseArea {
                                id: topDisconnectMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: wifiMenu.disconnectCurrentNetwork()
                            }
                        }

                        Rectangle {
                            id: forgetButton
                            property bool isEnabled: wifiMenu.activeSsid.length > 0
                            Layout.preferredHeight: 30
                            Layout.preferredWidth: 38
                            radius: 6
                            color: !isEnabled ? "#23232d" : (forgetMouse.containsMouse ? "#3a1f27" : "#2b1720")
                            border.color: Theme.outlineVariant
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.5

                            StyledIconToolTip {
                                visible: forgetMouse.containsMouse
                                text: "Forget"
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰆴"
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 12
                            }

                            MouseArea {
                                id: forgetMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                enabled: parent.isEnabled
                                onClicked: wifiMenu.forgetCurrentNetwork()
                            }
                        }
                    }

                    Text {
                        text: "State: " + (wifiMenu.activeState ? wifiMenu.activeState : (wifiMenu.wifiEnabled ? "idle" : "wifi disabled"))
                        color: Theme.secondary
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Signal: " + (wifiMenu.activeSignal >= 0 ? wifiMenu.signalBars(wifiMenu.activeSignal) : "N/A")
                        color: Theme.secondary
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Security: " + (wifiMenu.activeSecurity ? wifiMenu.activeSecurity : "N/A")
                        color: Theme.secondary
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "IP: " + (wifiMenu.activeIp ? wifiMenu.activeIp : "N/A")
                        color: Theme.secondary
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Gateway: " + (wifiMenu.activeGateway ? wifiMenu.activeGateway : "N/A")
                        color: Theme.secondary
                        font.family: Theme.font
                        font.pixelSize: 12
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
                    border.color: Theme.outlineVariant
                    border.width: 1

                    StyledIconToolTip {
                        visible: refreshMouse.containsMouse
                        text: "Refresh"
                    }

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: "󰑐"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 13
                        }
                    }

                    MouseArea {
                        id: refreshMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: wifiMenu.refreshAll()
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: wifiMenu.networks.length + " networks" + (wifiMenu.hiddenDuplicateCount > 0 ? "  •  " + wifiMenu.hiddenDuplicateCount + " hidden" : "")
                    color: Theme.surfaceContainer
                    font.family: Theme.font
                    font.pixelSize: 11
                }

                RowLayout {
                    spacing: 6
                    visible: wifiMenu.listLoading

                    Rectangle {
                        width: 8
                        height: 8
                        radius: 4
                        color: Theme.primary
                        opacity: 0.35

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
                        color: Theme.primary
                        font.family: Theme.font
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
                    color: Theme.background
                    radius: 8
                    border.color: Theme.outlineVariant
                    border.width: 1

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

                                delegate: Rectangle {
                                    required property var modelData
                                    id: networkRow
                                    property bool shouldAnimateOnCreate: wifiMenu.animateRowsOnNextLoad

                                    width: parent.width
                                    height: 56
                                    radius: 6
                                    color: wifiMenu.selectedSsid === modelData.ssid ? Theme.surfaceVariant : Theme.surface
                                    border.color: modelData.inUse === "*" ? Theme.primary : Theme.outlineVariant
                                    border.width: 1
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

                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: {
                                            if (wifiMenu.selectedSsid === modelData.ssid) {
                                                wifiMenu.clearSelection();
                                            } else {
                                                wifiMenu.selectedSsid = modelData.ssid;
                                                wifiMenu.selectedSecurity = modelData.security;
                                                wifiMenu.selectedSignal = modelData.signal;
                                                wifiMenu.selectedInUse = modelData.inUse;
                                                wifiMenu.requirePasswordRetry = false;
                                                passwordInput.text = "";
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
                                                text: modelData.ssid
                                                color: Theme.on_Surface
                                                font.family: Theme.font
                                                font.pixelSize: 12
                                                elide: Text.ElideRight
                                            }

                                            Text {
                                                text: "Signal " + wifiMenu.signalBars(modelData.signal) + "  •  " + (modelData.security ? modelData.security : "Open") + (modelData.inUse === "*" ? "  •  Connected" : "")
                                                color: Theme.secondary
                                                font.family: Theme.font
                                                font.pixelSize: 10
                                                elide: Text.ElideRight
                                            }
                                        }

                                        Item {
                                            Layout.fillWidth: true
                                        }

                                        Text {
                                            text: "󰅂"
                                            color: Theme.surfaceContainer
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
                    Layout.preferredWidth: wifiMenu.hasSelection ? 250 : 0
                    Layout.fillHeight: true
                    color: Theme.surface
                    radius: 8
                    border.color: Theme.outlineVariant
                    border.width: 1
                    opacity: wifiMenu.hasSelection ? 1 : 0
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
                                text: "Selected Network"
                                color: Theme.on_Surface
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
                                border.color: Theme.outlineVariant
                                border.width: 1

                                RowLayout {
                                    anchors.centerIn: parent
                                    spacing: 4

                                    Text {
                                        text: "󰅖"
                                        color: Theme.on_Surface
                                        font.family: Theme.font
                                        font.pixelSize: 11
                                    }

                                    Text {
                                        text: "Hide"
                                        color: Theme.on_Surface
                                        font.family: Theme.font
                                        font.pixelSize: 10
                                        font.bold: true
                                    }
                                }

                                MouseArea {
                                    id: sideCloseMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: wifiMenu.clearSelection()
                                }
                            }
                        }


                        Item {
                            id: ssidTextClip
                            Layout.fillWidth: true
                            Layout.minimumHeight: ssidTextA.implicitHeight
                            implicitHeight: ssidTextA.implicitHeight
                            clip: true

                            property int marqueeGap: 24
                            property real scrollSpeed: 42
                            property bool titleOverflow: ssidTextA.implicitWidth > width
                            property real loopSpan: ssidTextA.implicitWidth + marqueeGap
                            property real tickerOffset: 0

                            Text {
                                id: ssidTextA
                                text: wifiMenu.selectedSsid
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 14
                                font.bold: true
                                elide: Text.ElideNone
                                wrapMode: Text.NoWrap
                                anchors.verticalCenter: parent.verticalCenter
                                x: ssidTextClip.titleOverflow ? ssidTextClip.tickerOffset : 0
                            } 

                            Text {
                                id: ssidTextB
                                text: "|  " + wifiMenu.selectedSsid
                                color: Theme.on_Surface
                                font.family: Theme.font
                                font.pixelSize: 14
                                font.bold: true
                                elide: Text.ElideNone
                                wrapMode: Text.NoWrap
                                anchors.verticalCenter: parent.verticalCenter
                                x: ssidTextClip.tickerOffset + ssidTextClip.loopSpan
                                visible: ssidTextClip.titleOverflow
                            } 

                            NumberAnimation {
                                id: ssidMarquee
                                target: ssidTextClip
                                property: "tickerOffset"
                                from: 0
                                to: -ssidTextClip.loopSpan
                                duration: Math.max(1, Math.round((ssidTextClip.loopSpan / ssidTextClip.scrollSpeed) * 1000))
                                easing.type: Easing.Linear
                                running: wifiMenu.visible && ssidTextClip.titleOverflow
                                loops: Animation.Infinite

                                onRunningChanged: {
                                    if (!running)
                                        ssidTextClip.tickerOffset = 0;
                                }
                            }

                            onTitleOverflowChanged: {
                                if (!titleOverflow)
                                    tickerOffset = 0;
                            }
                        }

                        Text {
                            text: "Security: " + (wifiMenu.selectedSecurity ? wifiMenu.selectedSecurity : "Open")
                            color: Theme.secondary
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Text {
                            text: "Signal: " + (wifiMenu.selectedSignal >= 0 ? wifiMenu.signalBars(wifiMenu.selectedSignal) : "N/A")
                            color: Theme.secondary
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Text {
                            text: "Status: " + (wifiMenu.selectedInUse === "*" ? "Connected" : "Available")
                            color: Theme.secondary
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Theme.outlineVariant
                            opacity: 0.8
                        }

                        Text {
                            visible: wifiMenu.shouldShowPasswordField()
                            text: "Network Password"
                            color: Theme.on_Surface
                            font.family: Theme.font
                            font.pixelSize: 11
                            font.bold: true
                        }

                        TextField {
                            id: passwordInput
                            visible: wifiMenu.shouldShowPasswordField()
                            Layout.fillWidth: true
                            placeholderText: "Enter password"
                            echoMode: TextInput.Password
                            enabled: visible
                            color: Theme.on_Surface
                            placeholderTextColor: Theme.surfaceBright
                            selectionColor: Theme.primary
                            selectedTextColor: Theme.surfaceContainerLowest

                            background: Rectangle {
                                radius: 6
                                color: passwordInput.enabled ? Theme.surface : Theme.surfaceDim
                                border.color: passwordInput.activeFocus ? Theme.primary : Theme.outlineVariant
                                border.width: 1

                                Behavior on border.color {
                                    ColorAnimation {
                                        duration: 120
                                    }
                                }
                            }
                        }

                        Rectangle {
                            property bool requiresPassword: wifiMenu.shouldShowPasswordField()
                            property bool hasPassword: passwordInput.text.trim().length > 0
                            property bool isEnabled: wifiMenu.selectedSsid.length > 0 && wifiMenu.wifiEnabled && wifiMenu.selectedInUse !== "*" && (!requiresPassword || hasPassword)
                            Layout.preferredHeight: 36
                            Layout.fillWidth: true
                            radius: 6
                            color: !isEnabled ? "#23232d" : (connectMainMouse.containsMouse ? "#29503a" : "#1f3e2c")
                            border.color: Theme.outlineVariant
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.6

                            RowLayout {
                                anchors.centerIn: parent
                                spacing: 6

                                Text {
                                    text: "󰖩"
                                    color: Theme.on_Surface
                                    font.family: Theme.font
                                    font.pixelSize: 13
                                }

                                Text {
                                    text: wifiMenu.selectedInUse === "*" ? "Connected" : "Connect"
                                    color: Theme.on_Surface
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
                                onClicked: wifiMenu.connectSelectedNetwork()
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
                text: wifiMenu.statusMessage
                color: Theme.tertiary
                font.family: Theme.font
                font.pixelSize: 11
                wrapMode: Text.Wrap
                visible: text.length > 0
            }
        }
    }
}
