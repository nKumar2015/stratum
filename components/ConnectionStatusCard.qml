pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"
import "../modules"

Rectangle {
    id: root

    property string headlineText: ""
    property bool actionEnabled: false
    property string actionIconText: ""
    property real actionIconPixelSize: 20
    property string actionTooltipText: ""

    property string firstLabel: ""
    property string firstValue: ""
    property color firstValueColor: Theme.palette.textMain

    property string secondLabel: ""
    property string secondValue: ""
    property color secondValueColor: Theme.palette.textMain

    property string thirdLabel: ""
    property string thirdValue: ""
    property color thirdValueColor: Theme.palette.textMain

    signal actionClicked(var mouse)

    Layout.fillWidth: true
    Layout.preferredHeight: 64
    color: Theme.palette.bgWidget
    radius: 8

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 0

        RowLayout {
            Layout.fillWidth: true

            Text {
                text: root.headlineText
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 13
                font.bold: true
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            CompactIconButton {
                id: actionButton
                Layout.preferredHeight: 30
                Layout.preferredWidth: 38
                buttonRadius: 6
                iconText: root.actionIconText
                iconPixelSize: root.actionIconPixelSize
                enabled: root.actionEnabled
                disabledBackgroundColor: Theme.palette.bgDark
                borderColor: "transparent"
                hoverBorderColor: "transparent"
                onClicked: mouse => root.actionClicked(mouse)

                StyledIconToolTip {
                    visible: actionButton.hovered && root.actionTooltipText.length > 0
                    text: root.actionTooltipText
                }
            }
        }

        TripleStatusStrip {
            firstLabel: root.firstLabel
            firstValue: root.firstValue
            firstLabelColor: Theme.palette.tertiary
            firstValueColor: root.firstValueColor

            secondLabel: root.secondLabel
            secondValue: root.secondValue
            secondLabelColor: Theme.palette.tertiary
            secondValueColor: root.secondValueColor

            thirdLabel: root.thirdLabel
            thirdValue: root.thirdValue
            thirdLabelColor: Theme.palette.tertiary
            thirdValueColor: root.thirdValueColor

            labelPixelSize: 12
            valuePixelSize: 12
            dotPixelSize: 10
            valueBold: false
            thirdValueFillWidth: false
        }
    }
}
