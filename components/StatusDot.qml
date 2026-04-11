pragma ComponentBehavior: Bound

import QtQuick

import "../globals"

Text {
    id: root

    property color dotColor: Theme.palette.textMain
    property real dotPixelSize: 10

    text: "•"
    color: dotColor
    font.family: Theme.palette.font
    font.pixelSize: dotPixelSize
    elide: Text.ElideRight
}
