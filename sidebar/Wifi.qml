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

    property string icon: "\udb82\udd2e"

    function updateStatus(output) {
        let raw = output.trim();
        if (raw === "ethernet") {
            icon = "\udb80\ude00";
        } else if (raw.startsWith("wifi:")) {
            let strength = parseInt(raw.split(":")[1]);
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
        command: ["sh", Quickshell.shellDir + "/scripts/check_net.sh"]
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
        color: wifiHover.containsMouse ? Theme.blue : Theme.text
        font.pixelSize: 20

        Behavior on color {
            ColorAnimation {
                duration: 150
            }
        }
    }

    MouseArea {
        id: wifiHover
        anchors.fill: parent
        hoverEnabled: true
        onClicked: GlobalState.showWifiSettings = !GlobalState.showWifiSettings
    }
}
