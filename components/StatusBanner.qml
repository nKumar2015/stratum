pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property string messageText: ""
    property bool isError: false
    property color successColor: Theme.palette.success
    property color errorColor: Theme.palette.error
    property color textColor: Theme.palette.textMain
    property real textPixelSize: 11
    property bool textBold: true
    property int bannerHeight: 28
    property int bannerRadius: 6

    Layout.fillWidth: true
    Layout.preferredHeight: bannerHeight
    visible: root.messageText.length > 0
    radius: bannerRadius
    color: root.isError ? root.errorColor : root.successColor

    Text {
        anchors.centerIn: parent
        text: root.messageText
        color: root.textColor
        font.family: Theme.palette.font
        font.pixelSize: root.textPixelSize
        font.bold: root.textBold
    }
}
