import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
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
        const iconY = GlobalState.audioIconY;
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
    implicitHeight: Math.max(col.implicitHeight + 24, 220)

    visible: GlobalState.showAudioHoverMenu
    color: "transparent"

    property bool loading: false
    property var outputDevices: []
    property var inputDevices: []
    property string defaultOutput: ""
    property string defaultInput: ""
    property string errorMsg: ""
    property string statusMsg: ""
    property bool switching: false
    property int currentVolume: 0
    property bool currentMuted: false
    property bool sliderSyncing: false
    property int pendingVolume: -1
    property int expectedVolume: -1
    property int expectedVolumeMisses: 0
    readonly property int volumeMaxPercent: 150
    readonly property int lowVolumeThreshold: 34
    readonly property int mediumVolumeThreshold: 67
    readonly property int expectedVolumeTolerance: 2
    readonly property int maxExpectedVolumeMisses: 2
    readonly property int statusRetryDelayMs: 120
    readonly property int statusClearDelayMs: 1800
    readonly property int hoverHideDelayMs: 350

    function volumeIconFor(volume, muted) {
        if (muted || volume === 0)
            return "󰖁";
        if (volume < lowVolumeThreshold)
            return "󰕿";
        if (volume < mediumVolumeThreshold)
            return "󰖀";
        return "󰕾";
    }

    function deviceLabel(name, description) {
        if (description && description.trim().length > 0)
            return description.trim();
        return name;
    }

    function loadStatus(showLoading) {
        if (showLoading === undefined)
            showLoading = true;
        loading = showLoading;
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

    function previewVolume(value) {
        const clamped = Math.max(0, Math.min(volumeMaxPercent, Math.round(value)));
        currentVolume = clamped;
        currentMuted = clamped === 0;
        GlobalState.audioVolumePercent = clamped;
        GlobalState.audioMuted = currentMuted;
    }

    function queueVolumeCommit(value) {
        pendingVolume = Math.max(0, Math.min(volumeMaxPercent, Math.round(value)));
        expectedVolume = pendingVolume;
        expectedVolumeMisses = 0;
        GlobalState.audioUserAdjusting = true;
        if (!volumeProc.running)
            commitPendingVolume();
    }

    function commitPendingVolume() {
        if (pendingVolume < 0)
            return;

        const value = pendingVolume;
        pendingVolume = -1;
        volumeProc.command = ["stratum-cli", "audio", "set-volume", String(value)];
        volumeProc.running = true;
    }

    function parseVolumeStatus(volumeText, muteText) {
        const parsedVolume = parseInt((volumeText || "0").replace("%", ""));
        const statusVolume = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(volumeMaxPercent, parsedVolume));
        const statusMuted = (muteText || "yes").trim().toLowerCase() === "yes";

        if (GlobalState.audioUserAdjusting && !volumeSlider.pressed && pendingVolume < 0)
            GlobalState.audioUserAdjusting = false;

        if (GlobalState.audioUserAdjusting || volumeSlider.pressed || pendingVolume >= 0)
            return;

        if (expectedVolume >= 0) {
            if (Math.abs(statusVolume - expectedVolume) > expectedVolumeTolerance && expectedVolumeMisses < maxExpectedVolumeMisses) {
                expectedVolumeMisses += 1;
                statusRetryTimer.restart();
                return;
            }
            expectedVolume = -1;
            expectedVolumeMisses = 0;
        }

        currentVolume = statusVolume;
        currentMuted = statusMuted;
        GlobalState.audioVolumePercent = currentVolume;
        GlobalState.audioMuted = currentMuted;

        if (!volumeSlider.pressed) {
            sliderSyncing = true;
            volumeSlider.value = currentVolume;
            sliderSyncing = false;
        }
    }

    function switchOutput(name) {
        if (!name || switching)
            return;
        switching = true;
        statusMsg = "Switching output...";
        actionProc.command = ["stratum-cli", "audio", "set-output", name];
        actionProc.running = true;
    }

    function switchInput(name) {
        if (!name || switching)
            return;
        switching = true;
        statusMsg = "Switching input...";
        actionProc.command = ["stratum-cli", "audio", "set-input", name];
        actionProc.running = true;
    }

    Process {
        id: statusProc
        command: ["stratum-cli", "audio", "status", "--hover"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;
                const raw = this.text.trim();
                if (!raw) {
                    hoverMenu.outputDevices = [];
                    hoverMenu.inputDevices = [];
                    return;
                }

                const payload = hoverMenu.parseCliJson(raw);
                if (!payload || payload.ok !== true) {
                    hoverMenu.errorMsg = payload && payload.error ? String(payload.error) : "Audio status unavailable";
                    hoverMenu.outputDevices = [];
                    hoverMenu.inputDevices = [];
                    return;
                }

                const status = payload.status || {};
                const defaults = payload.default || {};

                hoverMenu.sliderSyncing = true;
                hoverMenu.parseVolumeStatus(String(status.volume || "0%"), String(status.mute || "yes"));
                hoverMenu.sliderSyncing = false;

                hoverMenu.defaultOutput = String(defaults.sink || "");
                hoverMenu.defaultInput = String(defaults.source || "");

                const sinks = Array.isArray(payload.sinks) ? payload.sinks : [];
                const sources = Array.isArray(payload.sources) ? payload.sources : [];

                hoverMenu.outputDevices = sinks.filter(function(row) {
                    return !!String(row.name || "").trim();
                }).map(function(row) {
                    return {
                        name: String(row.name || "").trim(),
                        description: String(row.description || "").trim()
                    };
                }).slice(0, 6);

                hoverMenu.inputDevices = sources.filter(function(row) {
                    return !!String(row.name || "").trim();
                }).map(function(row) {
                    return {
                        name: String(row.name || "").trim(),
                        description: String(row.description || "").trim()
                    };
                }).slice(0, 6);
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.switching = false;
                const payload = hoverMenu.parseCliJson(result);
                if (!payload || payload.ok !== true) {
                    hoverMenu.statusMsg = "Switch failed";
                    statusClearTimer.restart();
                } else {
                    hoverMenu.statusMsg = "Switched";
                    statusClearTimer.restart();
                }
                hoverMenu.loadStatus(false);
            }
        }
    }

    Process {
        id: volumeProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = hoverMenu.parseCliJson(result);
                if (!payload || payload.ok !== true) {
                    hoverMenu.statusMsg = "Volume change failed";
                    hoverMenu.expectedVolume = -1;
                    hoverMenu.expectedVolumeMisses = 0;
                    GlobalState.audioUserAdjusting = false;
                    statusClearTimer.restart();
                    return;
                }

                if (hoverMenu.pendingVolume >= 0) {
                    hoverMenu.commitPendingVolume();
                    return;
                }

                if (!volumeSlider.pressed)
                    GlobalState.audioUserAdjusting = false;

                hoverMenu.loadStatus(false);
            }
        }
    }

    Process {
        id: openPavucontrolProc
        command: ["stratum-cli", "audio", "open-control"]
    }

    Timer {
        id: statusRetryTimer
        interval: hoverMenu.statusRetryDelayMs
        repeat: false
        onTriggered: hoverMenu.loadStatus(false)
    }

    Timer {
        id: statusClearTimer
        interval: hoverMenu.statusClearDelayMs
        repeat: false
        onTriggered: hoverMenu.statusMsg = ""
    }

    onVisibleChanged: {
        if (visible) {
            outputDevices = [];
            inputDevices = [];
            errorMsg = "";
            statusMsg = "";
            currentVolume = 0;
            currentMuted = false;
            pendingVolume = -1;
            expectedVolume = -1;
            expectedVolumeMisses = 0;
            GlobalState.audioUserAdjusting = false;
            loadStatus();
        } else {
            hideTimer.stop();
            pendingVolume = -1;
            expectedVolume = -1;
            expectedVolumeMisses = 0;
            statusRetryTimer.stop();
            GlobalState.audioUserAdjusting = false;
        }
    }

    Timer {
        id: hideTimer
        interval: hoverMenu.hoverHideDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!GlobalState.audioHoverIntent && !menuHover.hovered)
                GlobalState.showAudioHoverMenu = false;
        }
    }

    Connections {
        target: GlobalState
        function onAudioHoverIntentChanged() {
            if (GlobalState.audioHoverIntent)
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

        HoverHandler {
            id: menuHover
            onHoveredChanged: GlobalState.audioHoverIntent = hovered
        }

        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "󰕾  Audio"
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
                        onClicked: GlobalState.showAudioHoverMenu = false
                    }

                    Behavior on color { ColorAnimation { duration: 100 } }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
            }

            Text {
                visible: hoverMenu.loading
                text: "Loading devices..."
                color: Theme.on_Surface
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length > 0
                text: hoverMenu.errorMsg
                color: Theme.error
                font.pixelSize: 12
                font.family: Theme.font
            }

            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.primary
                font.pixelSize: 11
                font.family: Theme.font
                opacity: 0.9
            }

            Item {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                Layout.fillWidth: true
                Layout.preferredHeight: 28

                RowLayout {
                    anchors.fill: parent
                    spacing: 8

                    Text {
                        text: hoverMenu.volumeIconFor(hoverMenu.currentVolume, hoverMenu.currentMuted)
                        color: Theme.on_Surface
                        font.family: Theme.font
                        font.pixelSize: 13
                    }

                    Slider {
                        id: volumeSlider
                        from: 0
                        to: hoverMenu.volumeMaxPercent
                        value: currentVolume
                        enabled: !hoverMenu.switching
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter

                        background: Rectangle {
                            x: volumeSlider.leftPadding
                            y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                            width: volumeSlider.availableWidth
                            height: 4
                            radius: 2
                            color: Theme.outlineVariant

                            Rectangle {
                                width: volumeSlider.visualPosition * parent.width
                                height: parent.height
                                radius: 2
                                color: Theme.primary
                            }
                        }

                        handle: Rectangle {
                            x: volumeSlider.leftPadding + volumeSlider.visualPosition * (volumeSlider.availableWidth - width)
                            y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                            implicitWidth: 12
                            implicitHeight: 12
                            radius: 6
                            color: volumeSlider.pressed ? Theme.primary : Theme.on_Surface
                        }

                        onValueChanged: {
                            if (hoverMenu.sliderSyncing)
                                return;

                            hoverMenu.previewVolume(value);
                        }

                        onPressedChanged: {
                            if (pressed) {
                                GlobalState.audioUserAdjusting = true;
                            } else if (!hoverMenu.sliderSyncing) {
                                hoverMenu.queueVolumeCommit(value);
                            }
                        }
                    }

                    Text {
                        text: Math.round(volumeSlider.value) + "%"
                        color: Theme.on_Surface
                        font.family: Theme.font
                        font.pixelSize: 10
                        opacity: 0.75
                        horizontalAlignment: Text.AlignRight
                        Layout.preferredWidth: 34
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                opacity: 0.5
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰕾  Output"
                color: Theme.on_Surface
                opacity: 0.75
                font.pixelSize: 12
                font.family: Theme.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.outputDevices.length === 0
                text: "No output devices"
                color: Theme.on_Surface
                opacity: 0.45
                font.pixelSize: 11
                font.family: Theme.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.outputDevices : []

                delegate: Rectangle {
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultOutput
                    color: outputHover.containsMouse ? Theme.outlineVariant : "transparent"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: selected ? "◉" : "○"
                            color: selected ? Theme.primary : Theme.on_Surface
                            font.pixelSize: 12
                            font.family: Theme.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(modelData.name, modelData.description)
                            color: Theme.on_Surface
                            font.pixelSize: 11
                            font.family: Theme.font
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    MouseArea {
                        id: outputHover
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !hoverMenu.switching
                        cursorShape: Qt.PointingHandCursor
                        onClicked: hoverMenu.switchOutput(modelData.name)
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                opacity: 0.5
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰍬  Input"
                color: Theme.on_Surface
                opacity: 0.75
                font.pixelSize: 12
                font.family: Theme.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.inputDevices.length === 0
                text: "No input devices"
                color: Theme.on_Surface
                opacity: 0.45
                font.pixelSize: 11
                font.family: Theme.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.inputDevices : []

                delegate: Rectangle {
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultInput
                    color: inputHover.containsMouse ? Theme.outlineVariant : "transparent"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: selected ? "◉" : "○"
                            color: selected ? Theme.secondary : Theme.on_Surface
                            font.pixelSize: 12
                            font.family: Theme.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(modelData.name, modelData.description)
                            color: Theme.on_Surface
                            font.pixelSize: 11
                            font.family: Theme.font
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    MouseArea {
                        id: inputHover
                        anchors.fill: parent
                        hoverEnabled: true
                        enabled: !hoverMenu.switching
                        cursorShape: Qt.PointingHandCursor
                        onClicked: hoverMenu.switchInput(modelData.name)
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.outlineVariant
            }

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
                        GlobalState.audioHoverIntent = false;
                        openPavucontrolProc.running = true;
                        GlobalState.showAudioHoverMenu = false;
                    }
                }

                Behavior on color { ColorAnimation { duration: 100 } }
            }
        }
        
    }
}
