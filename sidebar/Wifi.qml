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
        const state = String(WifiState.state || "disconnected").trim().toLowerCase();
        // Prefer the explicit ethernet flag from the global state (covers
        // cases where the textual `state` may not be exactly "ethernet").
        if (WifiState.ethernet || state === "ethernet") {
            icon = "\udb80\ude00";
            return;
        }

        if (state === "wifi") {
            const strength = Math.max(0, Math.min(100, Number(WifiState.signalPercent) || 0));
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
        target: WifiState
        function onStateChanged() {
            root.syncIconFromGlobalState();
        }
        function onSignalPercentChanged() {
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
            if (wifiHover.containsMouse && !WifiState.showMenu)
                WifiState.showHoverMenu = true;
        }
    }

    Timer {
        id: hoverExitGraceTimer
        interval: root.hoverExitGraceMs
        repeat: false
        onTriggered: {
            if (!wifiHover.containsMouse)
                WifiState.hoverIntent = false;
        }
    }

    MouseArea {
        id: wifiHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            hoverExitGraceTimer.stop();
            GlobalState.setPopupMonitorName(root.monitorName);
            WifiState.iconY = root.mapToGlobal(0, root.height / 2).y;
            WifiState.hoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            hoverShowTimer.stop();
            hoverExitGraceTimer.restart();
        }
        onClicked: {
            console.log("[Sidebar][Wifi] Icon clicked. Monitor: " + root.monitorName + ", New State: " + !WifiState.showMenu);
            hoverExitGraceTimer.stop();
            WifiState.hoverIntent = false;
            hoverShowTimer.stop();
            WifiState.showHoverMenu = false;
            WifiState.showMenu = !WifiState.showMenu;
        }
    }
}
