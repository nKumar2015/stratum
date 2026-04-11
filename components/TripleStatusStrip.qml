pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

RowLayout {
    id: root

    property string firstLabel: ""
    property string firstValue: ""
    property color firstLabelColor: Theme.palette.tertiary
    property color firstValueColor: Theme.palette.textMain

    property string secondLabel: ""
    property string secondValue: ""
    property color secondLabelColor: Theme.palette.tertiary
    property color secondValueColor: Theme.palette.textMain

    property string thirdLabel: ""
    property string thirdValue: ""
    property color thirdLabelColor: Theme.palette.tertiary
    property color thirdValueColor: Theme.palette.textMain

    property real labelPixelSize: 11
    property real valuePixelSize: 11
    property real dotPixelSize: 11
    property bool valueBold: false
    property bool thirdValueFillWidth: true

    Layout.fillWidth: true
    spacing: 5

    StatusRow {
        labelText: root.firstLabel
        valueText: root.firstValue
        labelColor: root.firstLabelColor
        valueColor: root.firstValueColor
        labelPixelSize: root.labelPixelSize
        valuePixelSize: root.valuePixelSize
        valueBold: root.valueBold
        valueFillWidth: false
    }

    StatusDot {
        dotPixelSize: root.dotPixelSize
    }

    StatusRow {
        labelText: root.secondLabel
        valueText: root.secondValue
        labelColor: root.secondLabelColor
        valueColor: root.secondValueColor
        labelPixelSize: root.labelPixelSize
        valuePixelSize: root.valuePixelSize
        valueBold: root.valueBold
        valueFillWidth: false
    }

    StatusDot {
        dotPixelSize: root.dotPixelSize
    }

    StatusRow {
        Layout.fillWidth: root.thirdValueFillWidth
        labelText: root.thirdLabel
        valueText: root.thirdValue
        labelColor: root.thirdLabelColor
        valueColor: root.thirdValueColor
        labelPixelSize: root.labelPixelSize
        valuePixelSize: root.valuePixelSize
        valueBold: root.valueBold
    }
}
