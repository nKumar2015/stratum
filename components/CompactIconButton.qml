pragma ComponentBehavior: Bound

import QtQuick
import Quickshell.Widgets

import "../globals"

Rectangle {
    id: root

    property string iconText: ""
    property string iconFamily: Theme.palette.font
    property url iconSource: ""
    property color iconColor: Theme.palette.textMain
    property color backgroundColor: Theme.palette.bgWidget
    property color hoverBackgroundColor: Theme.palette.bgHover
    property color borderColor: Theme.palette.borderInactive
    property color hoverBorderColor: Theme.palette.borderActive
    property color disabledBackgroundColor: backgroundColor
    property color disabledBorderColor: borderColor
    property real iconPixelSize: 13
    property real iconImageWidth: 18
    property real iconImageHeight: 18
    property int buttonWidth: 30
    property int buttonHeight: 24
    property int buttonRadius: 6
    readonly property bool hovered: mouseArea.containsMouse

    signal clicked(var mouse)

    implicitWidth: buttonWidth
    implicitHeight: buttonHeight
    radius: buttonRadius
    color: !enabled ? disabledBackgroundColor : (mouseArea.containsMouse ? hoverBackgroundColor : backgroundColor)
    border.width: 1
    border.color: !enabled ? disabledBorderColor : (mouseArea.containsMouse ? hoverBorderColor : borderColor)
    opacity: enabled ? 1 : 0.5

    Text {
        anchors.centerIn: parent
        visible: root.iconText.length > 0
        text: root.iconText
        color: root.iconColor
        font.pixelSize: root.iconPixelSize
        font.bold: true
        font.family: root.iconFamily
    }

    IconImage {
        anchors.centerIn: parent
        visible: root.iconText.length === 0 && String(root.iconSource || "").length > 0
        implicitWidth: root.iconImageWidth
        implicitHeight: root.iconImageHeight
        source: root.iconSource
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        enabled: root.enabled
        acceptedButtons: Qt.LeftButton | Qt.RightButton

        onClicked: mouse => root.clicked(mouse)
    }
}