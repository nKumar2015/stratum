pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Text {
    id: root

    property string messageText: ""
    property color messageColor: Theme.palette.tertiary
    property real messagePixelSize: 11

    Layout.fillWidth: true
    text: root.messageText
    color: root.messageColor
    font.family: Theme.palette.font
    font.pixelSize: root.messagePixelSize
    wrapMode: Text.Wrap
    visible: text.length > 0
}
