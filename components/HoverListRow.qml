pragma ComponentBehavior: Bound

import QtQuick

import "../globals"

Rectangle {
    id: root
    default property alias contentData: contentHost.data

    property string labelText: ""
    property bool isActive: false
    property bool isInteractive: true
    property bool highlightOnHover: true
    property bool showLabel: true
    property int rowHeight: 42
    property int rowRadius: 8
    property int contentLeftMargin: 0
    property int contentRightMargin: 0
    property real labelPixelSize: 13
    property bool labelBoldWhenActive: true
    property color labelColor: Theme.palette.textMain
    property color backgroundColor: Theme.palette.bgWidget
    property color activeBackgroundColor: Theme.palette.bgHover
    property color borderColor: Theme.palette.borderInactive
    property color activeBorderColor: Theme.palette.borderActive

    signal clicked(var mouse)
    signal hoveredChanged(bool hovered)

    implicitHeight: rowHeight
    radius: rowRadius
    color: (isActive || (highlightOnHover && mouseArea.containsMouse)) ? activeBackgroundColor : backgroundColor
    border.width: 1
    border.color: (isActive || (highlightOnHover && mouseArea.containsMouse)) ? activeBorderColor : borderColor
    opacity: isInteractive ? 1 : 0.6

    Item {
        id: contentHost
        anchors.fill: parent
        anchors.leftMargin: root.contentLeftMargin
        anchors.rightMargin: root.contentRightMargin

        Text {
            anchors.centerIn: parent
            visible: root.showLabel
            text: root.labelText
            color: root.labelColor
            font.pixelSize: root.labelPixelSize
            font.bold: root.labelBoldWhenActive && root.isActive
            font.family: Theme.palette.font
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        enabled: root.isInteractive
        cursorShape: root.isInteractive ? Qt.PointingHandCursor : Qt.ArrowCursor

        onContainsMouseChanged: root.hoveredChanged(containsMouse)
        onClicked: mouse => root.clicked(mouse)
    }
}