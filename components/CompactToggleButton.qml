pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property string iconText: ""
    property string labelText: ""
    property color iconColor: Theme.palette.textMain
    property color labelColor: Theme.palette.textMain
    property color backgroundColor: Theme.palette.bgWidget
    property color hoverBackgroundColor: Theme.palette.bgHover
    property color disabledBackgroundColor: Theme.palette.bgDark
    property color borderColor: Theme.palette.borderInactive
    property color hoverBorderColor: Theme.palette.borderActive
    property color disabledBorderColor: Theme.palette.borderInactive
    property int buttonHeight: 32
    property int buttonWidth: 98
    property int buttonRadius: 6
    property int contentSpacing: 6
    property real iconPixelSize: 13
    property real labelPixelSize: 12

    signal clicked(var mouse)
    signal hoveredChanged(bool hovered)

    implicitHeight: buttonHeight
    implicitWidth: buttonWidth
    radius: buttonRadius
    color: !enabled ? disabledBackgroundColor : (mouseArea.containsMouse ? hoverBackgroundColor : backgroundColor)
    border.width: 1
    border.color: !enabled ? disabledBorderColor : (mouseArea.containsMouse ? hoverBorderColor : borderColor)
    opacity: enabled ? 1 : 0.6

    RowLayout {
        anchors.centerIn: parent
        spacing: root.contentSpacing

        Text {
            text: root.iconText
            color: root.iconColor
            font.family: Theme.palette.font
            font.pixelSize: root.iconPixelSize
        }

        Text {
            text: root.labelText
            color: root.labelColor
            font.family: Theme.palette.font
            font.pixelSize: root.labelPixelSize
            font.bold: true
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        enabled: root.enabled
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor

        onContainsMouseChanged: root.hoveredChanged(containsMouse)
        onClicked: mouse => root.clicked(mouse)
    }
}