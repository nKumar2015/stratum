pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

RowLayout {
    id: root

    property string titleText: "Selected"
    property color titleColor: Theme.palette.textMuted
    property real titlePixelSize: 12
    property bool titleBold: true

    signal closeClicked(var mouse)

    Layout.fillWidth: true

    Text {
        text: root.titleText
        color: root.titleColor
        font.family: Theme.palette.font
        font.pixelSize: root.titlePixelSize
        font.bold: root.titleBold
    }

    Item {
        Layout.fillWidth: true
    }

    CompactIconButton {
        Layout.preferredWidth: 30
        Layout.preferredHeight: 28
        buttonRadius: 5
        iconText: "󰅖"
        iconColor: Theme.palette.error
        iconPixelSize: 16
        borderColor: "transparent"
        hoverBorderColor: "transparent"
        onClicked: mouse => root.closeClicked(mouse)
    }
}
