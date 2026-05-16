import QtQuick
import QtQuick.Layouts
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
        else if (volumePercent >= 100)
            icon = "󰕾";
        else
            icon = "󰝞";
    }

    Connections {
        target: AudioState
        function onVolumePercentChanged() {
            volumePercent = Math.max(0, Math.min(volumeMaxPercent, AudioState.volumePercent));
            applyIconState();
        }
        function onMutedChanged() {
            muted = AudioState.muted;
            applyIconState();
        }
        function onHeadphonesOutputChanged() {
            headphonesOutput = AudioState.headphonesOutput;
            applyIconState();
        }
    }

    Component.onCompleted: MusicProvider.acquire()
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
                AudioState.showHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: 420
        repeat: false
        onTriggered: {
            if (!audioHover.containsMouse)
                AudioState.hoverIntent = false;
        }
    }

    MouseArea {
        id: audioHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            AudioState.iconY = root.mapToGlobal(0, root.height / 2).y;
            AudioState.hoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            console.log("[Sidebar][Audio] Icon clicked. Monitor: " + root.monitorName + ", New State: " + !AudioState.showMenu);
            hoverExitGraceTimer.stop();
            AudioState.hoverIntent = false;
            AudioState.showMenu = !AudioState.showMenu;
            AudioState.showHoverMenu = false;
        }
    }
}
