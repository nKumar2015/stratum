pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

RowLayout {
    id: root

    property string labelText: ""
    property string valueText: ""
    property string leadingIconText: ""
    property color labelColor: Theme.palette.textMuted
    property color valueColor: Theme.palette.textMain
    property color iconColor: Theme.palette.textMain
    property real labelPixelSize: 11
    property real valuePixelSize: 11
    property real iconPixelSize: 12
    property bool valueBold: true
    property bool valueFillWidth: true
    property bool valueElideRight: true

    spacing: 6

    Text {
        visible: root.leadingIconText.length > 0
        text: root.leadingIconText
        color: root.iconColor
        font.pixelSize: root.iconPixelSize
        font.family: Theme.palette.font
    }

    Text {
        text: root.labelText
        color: root.labelColor
        font.pixelSize: root.labelPixelSize
        font.family: Theme.palette.font
    }

    Text {
        text: root.valueText
        color: root.valueColor
        font.pixelSize: root.valuePixelSize
        font.family: Theme.palette.font
        font.bold: root.valueBold
        Layout.fillWidth: root.valueFillWidth
        elide: root.valueElideRight ? Text.ElideRight : Text.ElideNone
    }
}