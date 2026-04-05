import QtQuick
import Quickshell
import QtQuick.Layouts
import Quickshell.Io
import "../theme"
import "../globals"

Item {
    id: root

    property string monitorName: ""

    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 16
    Layout.preferredHeight: 16

    property string icon: "󰖀"
    property int volumePercent: 0
    property bool muted: false
    property bool headphonesOutput: false

    function applyIconState() {
        if (muted || volumePercent === 0)
            icon = "󰖁";
        else if (headphonesOutput)
            icon = "󰋋";
        else if (volumePercent < 34)
            icon = "󰕿";
        else if (volumePercent < 67)
            icon = "󰖀";
        else if (volumePercent <= 100)
            icon = "󰕾";
        else
            icon = "󰝞";
    }

    function parseCliJson(raw) {
        try {
            return JSON.parse(String(raw || "").trim());
        } catch (_error) {
            return null;
        }
    }

    function updateStatus(output) {
        if (GlobalState.audioUserAdjusting)
            return;

        const payload = parseCliJson(output);
        if (!payload || payload.ok !== true)
            return;

        const volumeText = String(payload.volume || "0%").trim();
        const muteText = String(payload.mute || "yes").trim().toLowerCase();
        const headphonesText = String(payload.headphones || "no").trim().toLowerCase();
        const parsedVolume = parseInt(volumeText.replace("%", ""));

        volumePercent = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(150, parsedVolume));
        muted = muteText === "yes";
        headphonesOutput = headphonesText === "yes";
        GlobalState.audioVolumePercent = volumePercent;
        GlobalState.audioMuted = muted;
        applyIconState();
    }

    Connections {
        target: GlobalState
        function onAudioVolumePercentChanged() {
            volumePercent = Math.max(0, Math.min(150, GlobalState.audioVolumePercent));
            applyIconState();
        }
        function onAudioMutedChanged() {
            muted = GlobalState.audioMuted;
            applyIconState();
        }
    }

    Process {
        id: audioProc
        command: ["stratum-cli", "audio", "status"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result)
                    root.updateStatus(result);
                refreshTimer.start();
            }
        }
    }

    Timer {
        id: refreshTimer
        interval: 2500
        repeat: false
        onTriggered: audioProc.running = true
    }

    Process {
        id: openPavucontrolProc
        command: ["stratum-cli", "audio", "open-control"]
    }

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: audioHover.containsMouse ? Theme.primary : Theme.on_Surface
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: 150
            }
        }
    }

    Timer {
        id: hoverShowTimer
        interval: 350
        repeat: false
        onTriggered: {
            if (audioHover.containsMouse)
                GlobalState.showAudioHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: 420
        repeat: false
        onTriggered: {
            if (!audioHover.containsMouse)
                GlobalState.audioHoverIntent = false;
        }
    }

    MouseArea {
        id: audioHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            GlobalState.audioIconY = root.mapToGlobal(0, root.height / 2).y;
            GlobalState.audioHoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            hoverExitGraceTimer.stop();
            GlobalState.audioHoverIntent = false;
            openPavucontrolProc.running = true;
            GlobalState.showAudioHoverMenu = false;
        }
    }
}
