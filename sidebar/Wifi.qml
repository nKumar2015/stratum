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

    property string icon: "\udb82\udd2e"

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

    function updateStatus(output) {
        const payload = parseCliJson(output);
        if (!payload || payload.ok !== true)
            return;

        const state = String(payload.state || "").trim().toLowerCase();
        if (state === "ethernet") {
            icon = "\udb80\ude00";
        } else if (state === "wifi") {
            let strength = parseInt(String(payload.signal_pct || "0"));
            if (isNaN(strength))
                strength = 0;
            if (strength >= 80)
                icon = "\udb82\udd28";
            else if (strength >= 60)
                icon = "\udb82\udd25";
            else if (strength >= 40)
                icon = "\udb82\udd22";
            else if (strength >= 20)
                icon = "\udb82\udd1f";
            else
                icon = "\udb82\udd2f";
        } else {
            icon = "\udb82\udd2e";
        }
    }

    Process {
        id: netProc
        command: ["stratum-cli", "net", "check"]
        running: true
        stdout: StdioCollector {
            onStreamFinished: {
                let result = this.text.trim();
                if (result) {
                    root.updateStatus(result);
                }
                refreshTimer.start();
            }
        }
    }

    Timer {
        id: refreshTimer
        interval: 3000
        repeat: false
        onTriggered: netProc.running = true
    }

    Text {
        anchors.centerIn: parent
        text: root.icon
        color: wifiHover.containsMouse ? Theme.primary : Theme.on_Surface
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
            if (wifiHover.containsMouse && !GlobalState.showWifiSettings)
                GlobalState.showWifiHoverMenu = true;
        }
    }

    MouseArea {
        id: wifiHover
        anchors.fill: parent
        hoverEnabled: true
        onEntered: {
            GlobalState.setPopupMonitorName(root.monitorName);
            GlobalState.wifiIconY = root.mapToGlobal(0, root.height / 2).y;
            GlobalState.wifiHoverIntent = true;
            hoverShowTimer.start();
        }
        onExited: {
            GlobalState.wifiHoverIntent = false;
            hoverShowTimer.stop();
        }
        onClicked: {
            hoverShowTimer.stop();
            GlobalState.showWifiHoverMenu = false;
            GlobalState.showWifiSettings = !GlobalState.showWifiSettings;
        }
    }
}
