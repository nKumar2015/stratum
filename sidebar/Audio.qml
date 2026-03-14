import QtQuick
import Quickshell
import QtQuick.Layouts
import Quickshell.Io
import "../theme"
import "../globals"

Item {
    id: root

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

    function updateStatus(output) {
        if (GlobalState.audioUserAdjusting)
            return;

        const raw = output.trim();
        const parts = raw.split("|");
        if (parts.length < 2)
            return;

        const volumeText = (parts[0] || "0%").trim();
        const muteText = (parts[1] || "yes").trim().toLowerCase();
        const headphonesText = (parts[2] || "no").trim().toLowerCase();
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
        command: ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "status"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result && !result.startsWith("__ERROR__"))
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
        command: ["sh", Quickshell.shellDir + "/scripts/audio_menu.sh", "open-control"]
    }

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: audioHover.containsMouse ? Theme.blue : Theme.text
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
