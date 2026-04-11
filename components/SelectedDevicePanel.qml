pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Item {
    id: root

    property string panelTitle: "Selected Device"
    property string selectedName: ""
    property string selectedMac: ""
    property bool selectedConnected: false
    property bool selectedTrusted: false
    property bool selectedPaired: false
    property bool hasSelection: false
    property bool actionInProgress: false

    signal closeClicked(var mouse)
    signal pairClicked(var mouse)
    signal connectClicked(var mouse)
    signal trustClicked(var mouse)

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 8

        SelectedPanelHeader {
            titleText: root.panelTitle
            onCloseClicked: mouse => root.closeClicked(mouse)
        }

        MarqueeLabel {
            labelText: root.selectedName
            labelColor: Theme.palette.textMain
            labelPixelSize: 14
            labelBold: true
            marqueeGap: 24
            scrollSpeed: 42
        }

        StatusRow {
            Layout.fillWidth: true
            labelText: "MAC"
            valueText: root.selectedMac
            labelColor: Theme.palette.tertiary
            valueColor: Theme.palette.secondary
            labelPixelSize: 11
            valuePixelSize: 11
            valueBold: false
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 5

            Text {
                text: root.selectedConnected ? "Connected" : "Disconnected"
                color: root.selectedConnected ? Theme.palette.success : Theme.palette.error
                font.family: Theme.palette.font
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            StatusDot {
                dotPixelSize: 11
            }

            Text {
                text: root.selectedTrusted ? "Trusted" : "Untrusted"
                color: root.selectedTrusted ? Theme.palette.success : Theme.palette.error
                font.family: Theme.palette.font
                font.pixelSize: 11
                elide: Text.ElideRight
            }

            StatusDot {
                dotPixelSize: 11
            }

            Text {
                Layout.fillWidth: true
                text: root.selectedPaired ? "Paired" : "Unpaired"
                color: root.selectedPaired ? Theme.palette.success : Theme.palette.error
                font.family: Theme.palette.font
                font.pixelSize: 11
                elide: Text.ElideRight
            }
        }

        Divider {}

        ActionRowButton {
            iconText: root.selectedPaired ? "" : "󰌹"
            labelText: root.selectedPaired ? "Remove" : "Pair"
            enabled: root.hasSelection && !root.actionInProgress
            onClicked: mouse => root.pairClicked(mouse)
        }

        ActionRowButton {
            iconText: "󰖩"
            labelText: root.selectedConnected ? "Disconnect" : "Connect"
            enabled: root.hasSelection && !root.actionInProgress
            onClicked: mouse => root.connectClicked(mouse)
        }

        ActionRowButton {
            iconText: root.selectedTrusted ? "󰦞" : "󰕥"
            labelText: root.selectedTrusted ? "Untrust" : "Trust"
            enabled: root.hasSelection && !root.actionInProgress
            onClicked: mouse => root.trustClicked(mouse)
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
