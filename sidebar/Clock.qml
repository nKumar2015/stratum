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
        color: Theme.palette.primary // Using your active workspace color for emphasis
        font {
            pixelSize: 18
            bold: true
            family: Theme.palette.font // Or your preferred monospace font
        }
    }

    Text {
        text: clockRoot.timeParts.length > 0 ? clockRoot.timeParts[1] : "--"
        color: Theme.palette.secondary
        font {
            pixelSize: 18
            bold: true
            family: Theme.palette.font
        }
    }

    Text {
        horizontalAlignment: Text.AlignHCenter
        text: clockRoot.timeParts.length > 0 ? clockRoot.timeParts[2] : "--"
        color: Theme.palette.tertiary
        font {
            pixelSize: 18
            bold: false
            family: Theme.palette.font
        }
    }

    TapHandler {
        onTapped: GlobalState.showDashboardMenu = true
    }
}
