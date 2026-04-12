import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "../globals/DaemonRpc.js" as DaemonRpc
import "../globals"

Item {
    id: root

    property string monitorName: ""

    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 16
    Layout.preferredHeight: 16

    readonly property int volumeMaxPercent: 150
    readonly property int volumeLowThreshold: 34
    readonly property int volumeMidThreshold: 67
    readonly property int iconFadeMs: 150
    readonly property bool preferDaemonBootstrap: true

    property string icon: "󰖀"
    property int volumePercent: 0
    property bool muted: false
    property bool headphonesOutput: false

    // muted/zero=󰖁, headphones=󰋋, low=󰕿, mid=󰖀, high=󰕾, boosted=󰝞
    function applyIconState() {
        if (muted || volumePercent === 0)
            icon = "󰖁";
        else if (headphonesOutput)
            icon = "󰋋";
        else if (volumePercent < volumeLowThreshold)
            icon = "󰕿";
        else if (volumePercent < volumeMidThreshold)
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

    function bootstrapStatus() {
        if (preferDaemonBootstrap && DaemonRpc.canUse())
            daemonAudioProc.running = true;
        else
            audioProc.running = true;
    }

    function updateStatus(output) {
        if (GlobalState.audioUserAdjusting)
            return;

        const payload = parseCliJson(output);
        if (!payload)
            return;

        let source = null;
        if (payload.jsonrpc === "2.0" && payload.result && payload.result.ok === true && payload.result.audio)
            source = payload.result.audio;
        else if (payload.ok === true)
            source = payload;

        if (!source)
            return;

        const volumeText = String(source.volume || "0%").trim();
        const muteText = String(source.mute || "yes").trim().toLowerCase();
        const headphonesText = String(source.headphones || "no").trim().toLowerCase();
        const parsedVolume = parseInt(volumeText.replace("%", ""));

        volumePercent = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(volumeMaxPercent, parsedVolume));
        muted = muteText === "yes";
        headphonesOutput = headphonesText === "yes";
        GlobalState.audioVolumePercent = volumePercent;
        GlobalState.audioMuted = muted;
        applyIconState();
    }

    Connections {
        target: GlobalState
        function onAudioVolumePercentChanged() {
            volumePercent = Math.max(0, Math.min(volumeMaxPercent, GlobalState.audioVolumePercent));
            applyIconState();
        }
        function onAudioMutedChanged() {
            muted = GlobalState.audioMuted;
            applyIconState();
        }
        function onAudioHeadphonesOutputChanged() {
            headphonesOutput = GlobalState.audioHeadphonesOutput;
            applyIconState();
        }
    }

    Process {
        id: daemonAudioProc
        command: DaemonRpc.command("audio.status", {})
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                const payload = root.parseCliJson(result);
                if (payload && payload.jsonrpc === "2.0" && payload.result && payload.result.ok === true) {
                    DaemonRpc.recordSuccess();
                    GlobalState.daemonAvailable = true;
                    root.updateStatus(result);
                } else {
                    DaemonRpc.recordFailure();
                    GlobalState.daemonAvailable = false;
                    audioProc.running = true;
                    return;
                }
            }
        }
    }

    Process {
        id: audioProc
        command: ["stratum-cli", "audio", "status"]
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (result)
                    root.updateStatus(result);
            }
        }
    }

    Process {
        id: openPavucontrolProc
        command: ["stratum-cli", "audio", "open-control"]
    }

    Component.onCompleted: MusicProvider.acquire()
    Component.onCompleted: root.bootstrapStatus()
    Component.onDestruction: MusicProvider.release()

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: audioHover.containsMouse ? Theme.palette.secondary : Theme.palette.textMain
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: root.iconFadeMs
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
            GlobalState.showAudioMenu = true;
            GlobalState.showAudioHoverMenu = false;
        }
    }
}
