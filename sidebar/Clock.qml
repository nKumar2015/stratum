import QtQuick
import QtQuick.Layouts
import "../theme"
import "../globals"

ColumnLayout {
    id: clockRoot
    spacing: -4 // Tighten the stack for a more stylistic look

    property var timeParts: new Date().toLocaleTimeString(Qt.locale(), "hh mm AP").split(" ")

    function updateTime() {
        let now = new Date();
        let raw = now.toLocaleTimeString(Qt.locale(), "hh mm AP");
        timeParts = raw.split(" ");

        timer.interval = (60 - now.getSeconds()) * 1000 - now.getMilliseconds();
        timer.restart();
    }

    Timer {
        id: timer
        interval: 1000 * 60
        repeat: true
        running: true
        onTriggered: clockRoot.updateTime()
    }

    Text {
        text: clockRoot.timeParts.length > 0 ? clockRoot.timeParts[0] : "--"
        color: Theme.activeWs // Using your active workspace color for emphasis
        font {
            pixelSize: 18
            bold: true
            family: Theme.font // Or your preferred monospace font
        }
    }

    Text {
        text: clockRoot.timeParts.length > 0 ? clockRoot.timeParts[1] : "--"
        color: Theme.defaultWs
        font {
            pixelSize: 18
            bold: true
            family: Theme.font
        }
    }

    Text {
        horizontalAlignment: Text.AlignHCenter
        text: clockRoot.timeParts.length > 0 ? clockRoot.timeParts[2] : "--"
        color: Theme.inactiveWs // Use a dimmer color so it doesn't distract
        font {
            pixelSize: 18
            bold: false
            family: Theme.font
        }
    }

    MouseArea {
        id: batteryHover
        anchors.fill: parent
        hoverEnabled: true
        onClicked: GlobalState.showDashboardMenu = true
    }
}
