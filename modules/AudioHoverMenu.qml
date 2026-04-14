pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
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

    anchors.left: true
    anchors.top: true

    margins.left: 44
    margins.top: {
        const iconY = AudioState.iconY;
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

    visible: AudioState.showHoverMenu
    color: "transparent"
    readonly property color hoverSurface: Theme.palette.bgWidget
    readonly property color hoverBorder: Theme.palette.outlineVariant || Qt.rgba(1, 1, 1, 0.14)

    property bool loading: false
    property var outputDevices: AudioState.outputDevices
    property var inputDevices: AudioState.inputDevices
    property string defaultOutput: AudioState.defaultOutput
    property string defaultInput: AudioState.defaultInput
    property string errorMsg: ""
    property string statusMsg: ""
    property bool switching: false
    property int currentVolume: AudioState.volumePercent
    property bool currentMuted: AudioState.muted
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
    readonly property int hoverHideDelayMs: 800

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
        // RPC loading is now deprecated in favor of IPC pushes to AudioState.
        // We only keep this as a no-op/fallback for consistency.
        if (showLoading === undefined)
            showLoading = true;
        loading = false;
        errorMsg = "";
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
        AudioState.volumePercent = clamped;
        AudioState.muted = currentMuted;
    }

    function queueVolumeCommit(value) {
        pendingVolume = Math.max(0, Math.min(volumeMaxPercent, Math.round(value)));
        expectedVolume = pendingVolume;
        expectedVolumeMisses = 0;
        AudioState.userAdjusting = true;
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

        if (AudioState.userAdjusting && !volumeSlider.pressed && pendingVolume < 0)
            AudioState.userAdjusting = false;

        if (AudioState.userAdjusting || volumeSlider.pressed || pendingVolume >= 0)
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
        AudioState.volumePercent = currentVolume;
        AudioState.muted = currentMuted;

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

        if (DaemonRpc.canUse()) {
            actionProc.command = DaemonRpc.command("audio.set_output", {
                target: name
            });
        } else {
            actionProc.command = ["stratum-cli", "audio", "set-output", name];
        }
        actionProc.running = true;
    }

    function switchInput(name) {
        if (!name || switching)
            return;
        switching = true;
        statusMsg = "Switching input...";

        if (DaemonRpc.canUse()) {
            actionProc.command = DaemonRpc.command("audio.set_input", {
                target: name
            });
        } else {
            actionProc.command = ["stratum-cli", "audio", "set-input", name];
        }
        actionProc.running = true;
    }

    // statusProc was removed as status is now pushed via IPC.

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.switching = false;
                let payload = hoverMenu.parseCliJson(result);
                if (payload && payload.result !== undefined) {
                    payload = payload.result;
                }

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
                let payload = hoverMenu.parseCliJson(result);
                if (payload && payload.result !== undefined) {
                    payload = payload.result;
                }

                if (!payload || payload.ok !== true) {
                    hoverMenu.statusMsg = "Volume change failed";
                    hoverMenu.expectedVolume = -1;
                    hoverMenu.expectedVolumeMisses = 0;
                    AudioState.userAdjusting = false;
                    statusClearTimer.restart();
                    return;
                }

                if (hoverMenu.pendingVolume >= 0) {
                    hoverMenu.commitPendingVolume();
                    return;
                }

                if (!volumeSlider.pressed)
                    AudioState.userAdjusting = false;

                hoverMenu.loadStatus(false);
            }
        }
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
            // Initial data is already present via IPC/AudioState
        } else {
            hideTimer.stop();
            pendingVolume = -1;
            expectedVolume = -1;
            expectedVolumeMisses = 0;
            statusRetryTimer.stop();
            AudioState.userAdjusting = false;
        }
    }

    Timer {
        id: hideTimer
        interval: hoverMenu.hoverHideDelayMs
        repeat: false
        running: false
        onTriggered: {
            if (!AudioState.hoverIntent && !menuHover.hovered)
                AudioState.showHoverMenu = false;
        }
    }

    Connections {
        target: AudioState
        function onHoverIntentChanged() {
            if (AudioState.hoverIntent)
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

        HoverHandler {
            id: menuHover
            onHoveredChanged: {
                AudioState.hoverIntent = hovered;
                if (!hovered)
                    hideTimer.restart();
            }
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
                        onClicked: AudioState.showHoverMenu = false
                    }

                    Behavior on color {
                        ColorAnimation {
                            duration: 100
                        }
                    }
                }
            }

            Divider {}

            Text {
                visible: hoverMenu.loading
                text: "Loading devices..."
                color: Theme.palette.textMain
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.palette.font
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length > 0
                text: hoverMenu.errorMsg
                color: Theme.palette.error
                font.pixelSize: 12
                font.family: Theme.palette.font
            }

            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.family: Theme.palette.font
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
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 13
                    }

                    Slider {
                        id: volumeSlider
                        from: 0
                        to: hoverMenu.volumeMaxPercent
                        value: hoverMenu.currentVolume
                        enabled: !hoverMenu.switching
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter

                        background: Rectangle {
                            x: volumeSlider.leftPadding
                            y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                            width: volumeSlider.availableWidth
                            height: 4
                            radius: 2
                            color: Theme.palette.bgHover

                            Rectangle {
                                width: volumeSlider.visualPosition * parent.width
                                height: parent.height
                                radius: 2
                                color: Theme.palette.primary
                            }
                        }

                        handle: Rectangle {
                            x: volumeSlider.leftPadding + volumeSlider.visualPosition * (volumeSlider.availableWidth - width)
                            y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                            implicitWidth: 12
                            implicitHeight: 12
                            radius: 6
                            color: Theme.palette.primary
                        }

                        onValueChanged: {
                            if (hoverMenu.sliderSyncing)
                                return;

                            hoverMenu.previewVolume(value);
                        }

                        onPressedChanged: {
                            if (pressed) {
                                AudioState.userAdjusting = true;
                            } else if (!hoverMenu.sliderSyncing) {
                                hoverMenu.queueVolumeCommit(value);
                            }
                        }
                    }

                    Text {
                        text: Math.round(volumeSlider.value) + "%"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        horizontalAlignment: Text.AlignRight
                        Layout.preferredWidth: 34
                    }
                }
            }

            Divider {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰕾  Output"
                color: Theme.palette.textMain
                font.pixelSize: 12
                font.family: Theme.palette.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.outputDevices.length === 0
                text: "No output devices"
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.family: Theme.palette.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.outputDevices : []

                delegate: Rectangle {
                    id: outputItem
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultOutput
                    color: outputHover.containsMouse ? Theme.palette.bgHover : "transparent"
                    border.color: outputHover.containsMouse ? Theme.palette.borderActive : "transparent"
                    border.width: 1

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: outputItem.selected ? "◉" : "○"
                            color: outputItem.selected ? Theme.palette.textMain : Theme.palette.textMuted
                            font.pixelSize: 12
                            font.family: Theme.palette.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(outputItem.modelData.name, outputItem.modelData.description)
                            color: Theme.palette.textMain
                            font.pixelSize: 11
                            font.family: Theme.palette.font
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
                        onClicked: hoverMenu.switchOutput(outputItem.modelData.name)
                    }
                }
            }

            Divider {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰍬  Input"
                color: Theme.palette.textMain
                font.pixelSize: 12
                font.family: Theme.palette.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.inputDevices.length === 0
                text: "No input devices"
                color: Theme.palette.textMain
                font.pixelSize: 11
                font.family: Theme.palette.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.inputDevices : []

                delegate: Rectangle {
                    id: inputItem
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultInput
                    color: inputHover.containsMouse ? Theme.palette.bgHover : "transparent"
                    border.color: inputHover.containsMouse ? Theme.palette.borderActive : "transparent"
                    border.width: 1

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: inputItem.selected ? "◉" : "○"
                            color: inputItem.selected ? Theme.palette.textMain : Theme.palette.textMuted
                            font.pixelSize: 12
                            font.family: Theme.palette.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(inputItem.modelData.name, inputItem.modelData.description)
                            color: Theme.palette.textMain
                            font.pixelSize: 11
                            font.family: Theme.palette.font
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
                        onClicked: hoverMenu.switchInput(inputItem.modelData.name)
                    }
                }
            }

            Divider {}

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
                        AudioState.hoverIntent = false;
                        AudioState.showMenu = true;
                        AudioState.showHoverMenu = false;
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
