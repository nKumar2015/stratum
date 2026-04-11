pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Item {
    id: root

    property string labelText: ""
    property color labelColor: Theme.palette.textMain
    property real labelPixelSize: 14
    property bool labelBold: true
    property int marqueeGap: 24
    property real scrollSpeed: 42

    Layout.fillWidth: true
    Layout.minimumHeight: textA.implicitHeight
    implicitHeight: textA.implicitHeight
    clip: true

    property bool titleOverflow: measureText.implicitWidth > width
    property real loopSpan: textA.implicitWidth + marqueeGap
    property real tickerOffset: 0

    readonly property string separator: measureText.implicitWidth > width ? "    | " : ""

    Text {
        id: measureText
        text: root.labelText
        color: root.labelColor
        font.family: Theme.palette.font
        font.pixelSize: root.labelPixelSize
        font.bold: root.labelBold
        elide: Text.ElideNone
        visible: false
    }

    Text {
        id: textA
        text: root.labelText + root.separator
        color: root.labelColor
        font.family: Theme.palette.font
        font.pixelSize: root.labelPixelSize
        font.bold: root.labelBold
        elide: Text.ElideNone
        wrapMode: Text.NoWrap
        anchors.verticalCenter: parent.verticalCenter
        x: root.titleOverflow ? root.tickerOffset : 0
    }

    Text {
        id: textB
        text: root.labelText + root.separator
        color: root.labelColor
        font.family: Theme.palette.font
        font.pixelSize: root.labelPixelSize
        font.bold: root.labelBold
        elide: Text.ElideNone
        wrapMode: Text.NoWrap
        anchors.verticalCenter: parent.verticalCenter
        x: root.tickerOffset + root.loopSpan
        visible: root.titleOverflow
    }

    NumberAnimation {
        id: marquee
        target: root
        property: "tickerOffset"
        from: 0
        to: -root.loopSpan
        duration: Math.round((root.loopSpan / root.scrollSpeed) * 1000)
        easing.type: Easing.Linear
        running: root.visible && root.titleOverflow
        loops: Animation.Infinite

        onRunningChanged: {
            if (!running)
                root.tickerOffset = 0;
        }
    }

    onTitleOverflowChanged: {
        if (!titleOverflow)
            tickerOffset = 0;
    }
}
