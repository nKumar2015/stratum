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

    function previewVolume(value) {
        const clamped = Math.max(0, Math.min(150, Math.round(value)));
        currentVolume = clamped;
        currentMuted = clamped === 0;
        GlobalState.audioVolumePercent = clamped;
        GlobalState.audioMuted = currentMuted;
    }

    function queueVolumeCommit(value) {
        pendingVolume = Math.max(0, Math.min(150, Math.round(value)));
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
        volumeProc.command = ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "set-volume", String(value)];
        volumeProc.running = true;
    }

    function parseVolumeStatus(volumeText, muteText) {
        const parsedVolume = parseInt((volumeText || "0").replace("%", ""));
        const statusVolume = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(150, parsedVolume));
        const statusMuted = (muteText || "yes").trim().toLowerCase() === "yes";

        if (GlobalState.audioUserAdjusting && !volumeSlider.pressed && pendingVolume < 0)
            GlobalState.audioUserAdjusting = false;

        if (GlobalState.audioUserAdjusting || volumeSlider.pressed || pendingVolume >= 0)
            return;

        if (expectedVolume >= 0) {
            if (Math.abs(statusVolume - expectedVolume) > 2 && expectedVolumeMisses < 2) {
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
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "set-output", name];
        actionProc.running = true;
    }

    function switchInput(name) {
        if (!name || switching)
            return;
        switching = true;
        statusMsg = "Switching input...";
        actionProc.command = ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "set-input", name];
        actionProc.running = true;
    }

    Process {
        id: statusProc
        command: ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "hover-status"]
        stdout: StdioCollector {
            onStreamFinished: {
                hoverMenu.loading = false;
                const raw = this.text.trim();
                if (!raw) {
                    hoverMenu.outputDevices = [];
                    hoverMenu.inputDevices = [];
                    return;
                }
                if (raw.startsWith("__ERROR__")) {
                    hoverMenu.errorMsg = raw.replace("__ERROR__|", "");
                    hoverMenu.outputDevices = [];
                    hoverMenu.inputDevices = [];
                    return;
                }

                const lines = raw.split("\n");
                const sinks = [];
                const sources = [];
                let defOut = "";
                let defIn = "";

                for (let i = 0; i < lines.length; i++) {
                    const line = lines[i].trim();
                    if (!line)
                        continue;
                    const parts = line.split("|");
                    const type = (parts[0] || "").trim();

                    if (type === "STATUS") {
                        hoverMenu.sliderSyncing = true;
                        hoverMenu.parseVolumeStatus((parts[1] || "0%"), (parts[2] || "yes"));
                        hoverMenu.sliderSyncing = false;
                    } else if (type === "DEFAULT") {
                        defOut = (parts[1] || "").trim();
                        defIn = (parts[2] || "").trim();
                    } else if (type === "SINK") {
                        const name = (parts[1] || "").trim();
                        if (!name)
                            continue;
                        sinks.push({
                            name: name,
                            description: (parts[2] || "").trim()
                        });
                    } else if (type === "SOURCE") {
                        const name = (parts[1] || "").trim();
                        if (!name)
                            continue;
                        sources.push({
                            name: name,
                            description: (parts[2] || "").trim()
                        });
                    }
                }

                hoverMenu.defaultOutput = defOut;
                hoverMenu.defaultInput = defIn;
                hoverMenu.outputDevices = sinks.slice(0, 6);
                hoverMenu.inputDevices = sources.slice(0, 6);
            }
        }
    }

    Process {
        id: actionProc
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                hoverMenu.switching = false;
                if (result.startsWith("__ERROR__") || result.toLowerCase().indexOf("failed") !== -1) {
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
                if (result.startsWith("__ERROR__") || result.toLowerCase().indexOf("failed") !== -1) {
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

    Timer {
        id: statusRetryTimer
        interval: 120
        repeat: false
        onTriggered: hoverMenu.loadStatus(false)
    }

    Timer {
        id: statusClearTimer
        interval: 1800
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
        interval: 350
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
        border.color: Theme.grey
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
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 13
                    font.bold: true
                }

                Item { Layout.fillWidth: true }

                Text {
                    text: "󰅖"
                    color: closeHover.containsMouse ? Theme.red : Theme.text
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
                color: Theme.grey
            }

            Text {
                visible: hoverMenu.loading
                text: "Loading devices..."
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 12
                font.family: Theme.font
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length > 0
                text: hoverMenu.errorMsg
                color: Theme.red
                font.pixelSize: 12
                font.family: Theme.font
            }

            Text {
                visible: hoverMenu.statusMsg.length > 0
                text: hoverMenu.statusMsg
                color: Theme.blue
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
                        text: hoverMenu.currentMuted || hoverMenu.currentVolume === 0 ? "󰖁" : (hoverMenu.currentVolume < 34 ? "󰕿" : (hoverMenu.currentVolume < 67 ? "󰖀" : "󰕾"))
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 13
                    }

                    Slider {
                        id: volumeSlider
                        from: 0
                        to: 150
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
                            color: Theme.grey

                            Rectangle {
                                width: volumeSlider.visualPosition * parent.width
                                height: parent.height
                                radius: 2
                                color: Theme.blue
                            }
                        }

                        handle: Rectangle {
                            x: volumeSlider.leftPadding + volumeSlider.visualPosition * (volumeSlider.availableWidth - width)
                            y: volumeSlider.topPadding + volumeSlider.availableHeight / 2 - height / 2
                            implicitWidth: 12
                            implicitHeight: 12
                            radius: 6
                            color: volumeSlider.pressed ? Theme.blue : Theme.text
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
                        color: Theme.text
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
                color: Theme.grey
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                opacity: 0.5
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰕾  Output"
                color: Theme.text
                opacity: 0.75
                font.pixelSize: 12
                font.family: Theme.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.outputDevices.length === 0
                text: "No output devices"
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 11
                font.family: Theme.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.outputDevices : []

                delegate: Rectangle {
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: index === 0 ? 0 : -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultOutput
                    color: outputHover.containsMouse ? Theme.grey : "transparent"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: selected ? "◉" : "○"
                            color: selected ? Theme.blue : Theme.text
                            font.pixelSize: 12
                            font.family: Theme.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(modelData.name, modelData.description)
                            color: Theme.text
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
                color: Theme.grey
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                opacity: 0.5
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0
                text: "󰍬  Input"
                color: Theme.text
                opacity: 0.75
                font.pixelSize: 12
                font.family: Theme.font
                font.bold: true
            }

            Text {
                visible: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 && hoverMenu.inputDevices.length === 0
                text: "No input devices"
                color: Theme.text
                opacity: 0.45
                font.pixelSize: 11
                font.family: Theme.font
            }

            Repeater {
                model: !hoverMenu.loading && hoverMenu.errorMsg.length === 0 ? hoverMenu.inputDevices : []

                delegate: Rectangle {
                    required property var modelData

                    Layout.fillWidth: true
                    Layout.topMargin: index === 0 ? 0 : -6
                    height: 30
                    radius: 6
                    property bool selected: modelData.name === hoverMenu.defaultInput
                    color: inputHover.containsMouse ? Theme.grey : "transparent"

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 8
                        anchors.rightMargin: 8
                        spacing: 8

                        Text {
                            text: selected ? "◉" : "○"
                            color: selected ? Theme.green : Theme.text
                            font.pixelSize: 12
                            font.family: Theme.font
                        }

                        Text {
                            text: hoverMenu.deviceLabel(modelData.name, modelData.description)
                            color: Theme.text
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
        }
    }
}
