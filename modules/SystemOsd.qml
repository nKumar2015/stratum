import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../globals"

PanelWindow {
    id: osd

    anchors.top: true
    anchors.bottom: true
    anchors.left: true
    anchors.right: true

    color: "transparent"
    visible: osdOpacity > 0.01
    exclusiveZone: -1

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    property string channel: "volume"
    property int value: 0
    property bool muted: false
    property real osdOpacity: 0
    property var channelConfig: ({
            "volume": {
                max: 150,
                title: "Volume",
                valueColor: Theme.palette.secondary,
                pollMs: 300
            },
            "brightness": {
                max: 100,
                title: "Brightness",
                valueColor: Theme.palette.tertiary,
                pollMs: 350
            }
        })

    property int lastVolume: -1
    property bool lastMuted: false
    property int lastBrightness: -1

    property bool volumeBaselineReady: false
    property bool brightnessBaselineReady: false

    property int maxValue: {
        const config = channelConfig[channel];
        return config ? config.max : 100;
    }
    property real progressRatio: Math.max(0, Math.min(1, value / Math.max(1, maxValue)))

    property string iconGlyph: {
        if (channel === "brightness") {
            if (value >= 80)
                return "󰃠";
            if (value >= 40)
                return "󰃟";
            return "󰃞";
        }

        if (muted || value === 0)
            return "󰖁";
        if (value < 34)
            return "󰕿";
        if (value < 67)
            return "󰖀";
        if (value <= 100)
            return "󰕾";
        return "󰝞";
    }

    property string titleText: {
        const config = channelConfig[channel];
        return config ? String(config.title) : "Volume";
    }
    property string valueText: {
        if (channel === "volume" && muted)
            return "Muted";
        return String(value) + "%";
    }

    function channelMax(kind) {
        const config = channelConfig[kind];
        return config ? config.max : 100;
    }

    function channelAccent(kind) {
        const config = channelConfig[kind];
        return config.valueColor;
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

    function showChannel(kind, newValue, isMuted) {
        channel = kind;
        value = Math.max(0, Math.min(channelMax(kind), newValue));
        muted = isMuted;

        hideTimer.restart();
        fadeOut.stop();
        fadeIn.start();
    }

    NumberAnimation {
        id: fadeIn
        target: osd
        property: "osdOpacity"
        to: 1
        duration: 130
    }

    NumberAnimation {
        id: fadeOut
        target: osd
        property: "osdOpacity"
        to: 0
        duration: 220
    }

    Timer {
        id: hideTimer
        interval: 1200
        repeat: false
        onTriggered: fadeOut.start()
    }

    Process {
        id: volumeProc
        command: ["stratum-cli", "osd", "volume"]
        stdout: StdioCollector {
            onStreamFinished: {
                const raw = this.text.trim();
                const payload = osd.parseCliJson(raw);
                if (!payload || payload.ok !== true)
                    return;

                const value = parseInt(String(payload.value || "0"));
                if (isNaN(value))
                    return;

                const volume = Math.max(0, Math.min(150, value));
                const muteValue = String(payload.mute || "yes").trim().toLowerCase();
                const isMuted = muteValue === "yes" || muteValue === "true";

                GlobalState.audioVolumePercent = volume;
                GlobalState.audioMuted = isMuted;

                const changed = (volume !== osd.lastVolume) || (isMuted !== osd.lastMuted);
                osd.lastVolume = volume;
                osd.lastMuted = isMuted;

                if (!osd.volumeBaselineReady) {
                    osd.volumeBaselineReady = true;
                    return;
                }

                if (changed)
                    osd.showChannel("volume", volume, isMuted);
            }
        }
    }

    Process {
        id: brightnessProc
        command: ["stratum-cli", "osd", "brightness"]
        stdout: StdioCollector {
            onStreamFinished: {
                const raw = this.text.trim();
                const payload = osd.parseCliJson(raw);
                if (!payload || payload.ok !== true)
                    return;

                const parsed = parseInt(String(payload.value || "0"));
                const brightness = isNaN(parsed) ? 0 : Math.max(0, Math.min(100, parsed));
                const changed = brightness !== osd.lastBrightness;
                osd.lastBrightness = brightness;

                if (!osd.brightnessBaselineReady) {
                    osd.brightnessBaselineReady = true;
                    return;
                }

                if (changed)
                    osd.showChannel("brightness", brightness, false);
            }
        }
    }

    Timer {
        id: volumePoll
        interval: osd.channelConfig.volume.pollMs
        repeat: true
        running: true
        onTriggered: {
            if (!volumeProc.running)
                volumeProc.running = true;
        }
    }

    Timer {
        id: brightnessPoll
        interval: osd.channelConfig.brightness.pollMs
        repeat: true
        running: true
        onTriggered: {
            if (!brightnessProc.running)
                brightnessProc.running = true;
        }
    }

    Rectangle {
        width: 320
        height: 94
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 72
        opacity: osd.osdOpacity
        radius: 12
        color: Qt.rgba(0.07, 0.07, 0.11, 0.95)
        border.color: Theme.palette.borderActive
        border.width: 1

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 14
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: 12

                Text {
                    text: osd.iconGlyph
                    color: Theme.palette.textMain
                    font.pixelSize: 22
                    font.family: Theme.palette.font
                }

                Text {
                    text: osd.titleText
                    color: Theme.palette.textMain
                    font.pixelSize: 13
                    font.bold: true
                    font.family: Theme.palette.font
                    Layout.fillWidth: true
                }

                Text {
                    text: osd.valueText
                    color: osd.channelAccent(osd.channel)
                    font.pixelSize: 12
                    font.bold: true
                    font.family: Theme.palette.font
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 9
                radius: 5
                color: Theme.palette.bgWidget

                Rectangle {
                    width: parent.width * osd.progressRatio
                    height: parent.height
                    radius: parent.radius
                    color: osd.channelAccent(osd.channel)

                    Behavior on width {
                        NumberAnimation {
                            duration: 120
                            easing.type: Easing.OutCubic
                        }
                    }
                }
            }
        }
    }
}
