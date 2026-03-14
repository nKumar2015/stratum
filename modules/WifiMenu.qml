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

        activeInfoProc.command = ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "active-info", activeDevice];
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

        const cmd = ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "connect", selectedSsid];
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

        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "disconnect", activeDevice];
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
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "forget", activeSsid];
        statusMessage = "Forgetting " + activeSsid + "...";
        actionProc.running = true;
    }

    function toggleWifiRadio() {
        const target = wifiEnabled ? "off" : "on";
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "toggle", target];
        pendingAction = "toggle";
        pendingActionTarget = target;
        statusMessage = wifiEnabled ? "Turning Wi-Fi off..." : "Turning Wi-Fi on...";
        actionProc.running = true;
    }

    Process {
        id: wifiStateProc
        command: ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "state"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result.startsWith("__ERROR__|")) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    wifiMenu.wifiEnabled = false;
                    return;
                }
                wifiMenu.wifiEnabled = result.toLowerCase().indexOf("enabled") !== -1;
            }
        }
    }

    Process {
        id: deviceStatusProc
        command: ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "device-status"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result.startsWith("__ERROR__|")) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    return;
                }

                const lines = result.length > 0 ? result.split("\n") : [];
                let currentDevice = "";
                let currentSsid = "";
                let currentState = "";

                for (let i = 0; i < lines.length; i++) {
                    const cols = wifiMenu.splitNmcliFields(lines[i], 4);
                    if (cols.length < 4)
                        continue;
                    const device = cols[0].trim();
                    const type = cols[1].trim().toLowerCase();
                    const state = cols[2].trim().toLowerCase();
                    const conn = cols[3].trim();

                    if (type === "wifi" && state.indexOf("connected") !== -1) {
                        currentDevice = device;
                        currentSsid = conn;
                        currentState = cols[2].trim();
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
        command: ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "known-connections"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result.startsWith("__ERROR__|"))
                    return;

                const rows = result.length > 0 ? result.split("\n") : [];
                const known = {};

                for (let i = 0; i < rows.length; i++) {
                    const cols = wifiMenu.splitNmcliFields(rows[i], 2);
                    if (cols.length < 2)
                        continue;

                    const name = cols[0].trim();
                    const type = cols[1].trim().toLowerCase();
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
        command: ["sh", Quickshell.shellDir + "/scripts/wifi_menu.sh", "list"]
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                wifiMenu.listLoading = false;
                if (result.startsWith("__ERROR__|")) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    wifiMenu.networks = [];
                    return;
                }

                const rows = result.length > 0 ? result.split("\n") : [];
                const dedupBySsid = {};
                let candidateCount = 0;

                for (let i = 0; i < rows.length; i++) {
                    const cols = wifiMenu.splitNmcliFields(rows[i], 4);
                    if (cols.length < 4)
                        continue;

                    const inUse = cols[0].trim();
                    const ssid = cols[1].trim();
                    if (!ssid)
                        continue;

                    const candidate = {
                        inUse: inUse,
                        ssid: ssid,
                        signal: parseInt(cols[2].trim() || "0"),
                        security: cols[3].trim()
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
                if (result.startsWith("__ERROR__|")) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                    return;
                }

                const lines = result.length > 0 ? result.split("\n") : [];
                let ip = "";
                let gateway = "";

                for (let i = 0; i < lines.length; i++) {
                    const cols = wifiMenu.splitNmcliFields(lines[i], 2);
                    if (cols.length < 2)
                        continue;
                    const key = cols[0].trim();
                    const value = cols[1].trim();

                    if (key === "IP4.ADDRESS[1]")
                        ip = value;
                    else if (key === "IP4.GATEWAY")
                        gateway = value;
                }

                wifiMenu.activeIp = ip;
                wifiMenu.activeGateway = gateway;
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result.startsWith("__ERROR__|")) {
                    wifiMenu.statusMessage = "nmcli is required for Wi-Fi controls.";
                } else if (result.length > 0 && result.toLowerCase().indexOf("error") !== -1) {
                    if (wifiMenu.pendingAction === "connect" && wifiMenu.pendingConnectWasKnown && wifiMenu.pendingConnectWasSecure && !wifiMenu.requirePasswordRetry) {
                        wifiMenu.requirePasswordRetry = true;
                        wifiMenu.statusMessage = "Saved credentials failed. Enter password and retry.";
                    } else {
                        wifiMenu.statusMessage = result;
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
                    color: wifiToggleMouse.containsMouse ? "#263244" : "#1b2333"
                    border.color: Theme.grey
                    border.width: 1

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: 6

                        Text {
                            text: wifiMenu.wifiEnabled ? "󰤨" : "󰤮"
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 13
                        }

                        Text {
                            text: wifiMenu.wifiEnabled ? "On" : "Off"
                            color: Theme.text
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
                    border.color: Theme.grey
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
                            color: Theme.text
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
                border.color: Theme.grey
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
                            color: Theme.text
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
                            border.color: Theme.grey
                            border.width: 1
                            opacity: isEnabled ? 1.0 : 0.5

                            StyledIconToolTip {
                                visible: forgetMouse.containsMouse
                                text: "Forget"
                            }

                            Text {
                                anchors.centerIn: parent
                                text: "󰆴"
                                color: Theme.text
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
                        color: Theme.hover
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Signal: " + (wifiMenu.activeSignal >= 0 ? wifiMenu.signalBars(wifiMenu.activeSignal) : "N/A")
                        color: Theme.hover
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Security: " + (wifiMenu.activeSecurity ? wifiMenu.activeSecurity : "N/A")
                        color: Theme.hover
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "IP: " + (wifiMenu.activeIp ? wifiMenu.activeIp : "N/A")
                        color: Theme.hover
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Text {
                        text: "Gateway: " + (wifiMenu.activeGateway ? wifiMenu.activeGateway : "N/A")
                        color: Theme.hover
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
                    border.color: Theme.grey
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
                            color: Theme.text
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
                    color: Theme.hover
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
                        color: Theme.activeWs
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
                        color: Theme.activeWs
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
                    border.color: Theme.grey
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
                                    color: wifiMenu.selectedSsid === modelData.ssid ? "#1d2434" : Theme.background
                                    border.color: modelData.inUse === "*" ? Theme.activeWs : Theme.grey
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
                                                color: Theme.text
                                                font.family: Theme.font
                                                font.pixelSize: 12
                                                elide: Text.ElideRight
                                            }

                                            Text {
                                                text: "Signal " + wifiMenu.signalBars(modelData.signal) + "  •  " + (modelData.security ? modelData.security : "Open") + (modelData.inUse === "*" ? "  •  Connected" : "")
                                                color: Theme.hover
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
                    Layout.preferredWidth: wifiMenu.hasSelection ? 250 : 0
                    Layout.fillHeight: true
                    color: "#141422"
                    radius: 8
                    border.color: Theme.grey
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
                                    onClicked: wifiMenu.clearSelection()
                                }
                            }
                        }

                        Text {
                            text: wifiMenu.selectedSsid
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 14
                            font.bold: true
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "Security: " + (wifiMenu.selectedSecurity ? wifiMenu.selectedSecurity : "Open")
                            color: Theme.hover
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Text {
                            text: "Signal: " + (wifiMenu.selectedSignal >= 0 ? wifiMenu.signalBars(wifiMenu.selectedSignal) : "N/A")
                            color: Theme.hover
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Text {
                            text: "Status: " + (wifiMenu.selectedInUse === "*" ? "Connected" : "Available")
                            color: wifiMenu.selectedInUse === "*" ? Theme.green : Theme.hover
                            font.family: Theme.font
                            font.pixelSize: 11
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 1
                            color: Theme.grey
                            opacity: 0.8
                        }

                        Text {
                            visible: wifiMenu.shouldShowPasswordField()
                            text: "Network Password"
                            color: Theme.text
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
                            color: Theme.text
                            placeholderTextColor: Theme.hover
                            selectionColor: Theme.activeWs
                            selectedTextColor: Theme.black

                            background: Rectangle {
                                radius: 6
                                color: passwordInput.enabled ? "#10101b" : "#1f1f29"
                                border.color: passwordInput.activeFocus ? Theme.activeWs : Theme.grey
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
                                    text: wifiMenu.selectedInUse === "*" ? "Connected" : "Connect"
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
                color: Theme.yellow
                font.family: Theme.font
                font.pixelSize: 11
                wrapMode: Text.Wrap
                visible: text.length > 0
            }
        }
    }
}
