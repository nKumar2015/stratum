import QtQuick
import QtQuick.Layouts
import "../globals"

Item {
    id: root

    property string monitorName: ""

    Layout.alignment: Qt.AlignHCenter
    Layout.preferredWidth: 16
    Layout.preferredHeight: 16

    readonly property int signalHighThreshold: 80
    readonly property int signalMediumHighThreshold: 60
    readonly property int signalMediumThreshold: 40
    readonly property int signalLowThreshold: 20
    readonly property int iconFadeMs: 150
    readonly property int hoverOpenDelayMs: 350
    readonly property int hoverExitGraceMs: 420

    // disconnected=\udb82\udd2e, ethernet=\udb80\ude00, Wi-Fi weak->strong=\udb82\udd2f..\udb82\udd28
    property string icon: "\udb82\udd2e"

    function syncIconFromGlobalState() {
        const state = String(GlobalState.wifiState || "disconnected").trim().toLowerCase();
        if (state === "ethernet") {
            icon = "\udb80\ude00";
            return;
        }

        if (state === "wifi") {
            const strength = Math.max(0, Math.min(100, Number(GlobalState.wifiSignalPercent) || 0));
            if (strength >= signalHighThreshold)
                icon = "\udb82\udd28";
            else if (strength >= signalMediumHighThreshold)
                icon = "\udb82\udd25";
            else if (strength >= signalMediumThreshold)
                icon = "\udb82\udd22";
            else if (strength >= signalLowThreshold)
                icon = "\udb82\udd1f";
            else
                icon = "\udb82\udd2f";
            return;
        }

        icon = "\udb82\udd2e";
    }

    Connections {
        target: GlobalState
        function onWifiStateChanged() {
            root.syncIconFromGlobalState();
        }
        function onWifiSignalPercentChanged() {
            root.syncIconFromGlobalState();
        }
    }

    Component.onCompleted: root.syncIconFromGlobalState()

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: wifiHover.containsMouse ? Theme.palette.secondary : Theme.palette.textMain
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: root.iconFadeMs
            }
        }
    }

    Timer {
        id: hoverShowTimer
        interval: root.hoverOpenDelayMs
        repeat: false
        onTriggered: {
            if (wifiHover.containsMouse && !GlobalState.showWifiSettings)
                GlobalState.showWifiHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: root.hoverExitGraceMs
        repeat: false
        onTriggered: {
            if (!wifiHover.containsMouse)
                GlobalState.wifiHoverIntent = false;
        }
    }

    MouseArea {
        id: wifiHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            GlobalState.wifiIconY = root.mapToGlobal(0, root.height / 2).y;
            GlobalState.wifiHoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            hoverExitGraceTimer.stop();
            GlobalState.wifiHoverIntent = false;
            hoverShowTimer.stop();
            GlobalState.showWifiHoverMenu = false;
            GlobalState.showWifiSettings = !GlobalState.showWifiSettings;
        }
    }
}
