import Quickshell
import QtQuick

floatingWindow {
    visible: true
    width: 200
    height: 100

    Text {
        anchors.centerIn: parent
        text: "hello, Quickshell"
        color: "#0db9d7"
        font.pixelSize: 18
    }
}
