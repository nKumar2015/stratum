import QtQuick
import QtQuick.Layouts

import "../theme"

Rectangle {
    id: root

    property int percent: 0
    property color fillColor: Theme.secondary
    property bool animated: true

    Layout.fillWidth: true
    implicitHeight: 6
    radius: 3
    color: Theme.surfaceContainerLowest

    Rectangle {
        width: parent.width * Math.max(0, Math.min(100, root.percent)) / 100
        height: parent.height
        radius: 3
        color: root.fillColor

        Behavior on width {
            enabled: root.animated
            NumberAnimation {
                duration: 260
                easing.type: Easing.OutCubic
            }
        }
    }
}
