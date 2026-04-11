pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

import "../globals"

Item {
    id: root

    property string panelTitle: "Selected Network"
    property string selectedSsid: ""
    property string selectedInUse: ""
    property string selectedSignalDisplay: "N/A"
    property string selectedSecurity: ""
    property bool hasSelection: false
    property bool wifiEnabled: true
    property bool showPasswordField: false
    property bool canDisconnect: false
    property alias passwordText: passwordInput.text

    signal closeClicked(var mouse)
    signal connectClicked(var mouse)

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 10
        spacing: 8

        SelectedPanelHeader {
            titleText: root.panelTitle
            onCloseClicked: mouse => root.closeClicked(mouse)
        }

        MarqueeLabel {
            labelText: root.selectedSsid
            labelColor: Theme.palette.textMain
            labelPixelSize: 14
            labelBold: true
            marqueeGap: 24
            scrollSpeed: 42
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 5

            Text {
                text: root.selectedInUse === "*" ? "Connected" : "Disconnected"
                color: root.selectedInUse === "*" ? Theme.palette.success : Theme.palette.error
                font.family: Theme.palette.font
                font.pixelSize: 12
                elide: Text.ElideRight
            }

            StatusDot {
                dotPixelSize: 11
            }

            Text {
                text: root.selectedSignalDisplay
                color: root.selectedSignalDisplay === "N/A" ? Theme.palette.warning : Theme.palette.success
                font.family: Theme.palette.font
                font.pixelSize: 12
                elide: Text.ElideRight
            }

            StatusDot {
                dotPixelSize: 11
            }

            Text {
                Layout.fillWidth: true
                text: root.selectedSecurity ? root.selectedSecurity : "N/A"
                color: Theme.palette.tertiary
                font.family: Theme.palette.font
                font.pixelSize: 12
                elide: Text.ElideRight
            }
        }

        Divider {}

        Text {
            visible: root.showPasswordField
            text: "Network Password"
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 11
            font.bold: true
        }

        TextField {
            id: passwordInput
            visible: root.showPasswordField
            Layout.fillWidth: true
            placeholderText: "Enter password"
            echoMode: TextInput.Password
            enabled: visible
            color: Theme.palette.textMain
            placeholderTextColor: Theme.palette.textMuted
            selectionColor: Theme.palette.textMain
            selectedTextColor: Theme.palette.primary
            font.family: Theme.palette.font

            background: Rectangle {
                radius: 6
                color: passwordInput.enabled ? Theme.palette.bgMain : Theme.palette.bgDark
                border.color: passwordInput.activeFocus ? Theme.palette.borderActive : Theme.palette.borderInactive
                border.width: 1

                Behavior on border.color {
                    ColorAnimation {
                        duration: 120
                    }
                }
            }
        }

        ActionRowButton {
            id: connectAction
            property bool hasPassword: passwordInput.text.trim().length > 0
            property bool canConnect: root.hasSelection && root.wifiEnabled && root.selectedInUse !== "*" && (!root.showPasswordField || hasPassword)
            iconText: "󰖩"
            labelText: root.selectedInUse === "*" ? "Disconnect" : "Connect"
            enabled: root.selectedInUse === "*" ? root.canDisconnect : canConnect
            onClicked: mouse => root.connectClicked(mouse)
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
