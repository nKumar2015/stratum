pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../globals"

PanelWindow {
    id: polkitWindow

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    color: "#cc000000"
    visible: PolkitState.active

    property bool isError: false

    Connections {
        target: PolkitState
        function onAuthFinished(success) {
            if (!success) {
                isError = true;
                passwordField.text = "";
                errorTimer.stop();
                errorTimer.start();
            }
        }
    }

    Timer {
        id: errorTimer
        interval: 1500
        onTriggered: polkitWindow.isError = false
    }

    Rectangle {
        id: dialog
        width: 450
        height: 380
        anchors.centerIn: parent
        color: Theme.palette.bgMain
        radius: 16
        border.color: polkitWindow.isError ? Theme.palette.error : (dialogMouse.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive)
        border.width: 1

        Behavior on border.color {
            ColorAnimation {
                duration: 200
            }
        }

        MouseArea {
            id: dialogMouse
            anchors.fill: parent
            hoverEnabled: true
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 32
            spacing: 24

            RowLayout {
                spacing: 20

                Text {
                    text: "󱗼"
                    font.pixelSize: 48
                    color: Theme.palette.primary
                }

                ColumnLayout {
                    spacing: 4
                    Layout.fillWidth: true
                    Text {
                        text: "Authentication Required"
                        color: Theme.palette.textMain
                        font.pixelSize: 20
                        font.bold: true
                    }
                    Text {
                        text: PolkitState.action || "System action requires elevation."
                        color: Theme.palette.textMuted
                        font.pixelSize: 13
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                    }
                }
            }

            Text {
                text: PolkitState.message || "An application is attempting to perform an action that requires privileges. Please enter your password to continue."
                color: Theme.palette.textMain
                font.pixelSize: 14
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
            }

            ColumnLayout {
                spacing: 8
                Layout.fillWidth: true

                RowLayout {
                    Layout.fillWidth: true
                    Text {
                        text: "Password for " + PolkitState.user
                        color: Theme.palette.textMuted
                        font.pixelSize: 12
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    Text {
                        visible: polkitWindow.isError
                        text: PolkitState.message || "Incorrect Password"
                        color: Theme.palette.error
                        font.pixelSize: 12
                        font.bold: true
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                    }
                }

                TextField {
                    id: passwordField
                    Layout.fillWidth: true
                    placeholderText: "Enter password..."
                    echoMode: TextInput.Password
                    focus: true
                    enabled: !PolkitState.verifying

                    background: Rectangle {
                        color: Theme.palette.bgWidget
                        radius: 8
                        border.color: passwordField.activeFocus ? Theme.palette.primary : Theme.palette.borderInactive
                        border.width: 1
                    }

                    color: Theme.palette.textMain
                    font.pixelSize: 14

                    onAccepted: PolkitState.respond(text)
                }
            }

            RowLayout {
                Layout.alignment: Qt.AlignRight
                spacing: 12

                Button {
                    id: cancelButton
                    text: "Cancel"
                    enabled: !PolkitState.verifying
                    onClicked: PolkitState.cancel()

                    background: Rectangle {
                        implicitWidth: 80
                        implicitHeight: 36
                        color: cancelButton.hovered ? Theme.palette.bgHover : "transparent"
                        radius: 8
                        border.color: Theme.palette.borderInactive
                        border.width: 1
                    }

                    contentItem: Text {
                        text: parent.text
                        color: cancelButton.enabled ? Theme.palette.textMain : Theme.palette.textMuted
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                }

                Button {
                    id: authButton
                    text: PolkitState.verifying ? "Verifying..." : "Authenticate"
                    enabled: !PolkitState.verifying
                    onClicked: PolkitState.respond(passwordField.text)

                    background: Rectangle {
                        implicitWidth: 120
                        implicitHeight: 36
                        color: !authButton.enabled ? Theme.palette.bgWidget : (authButton.pressed ? Theme.palette.secondary : Theme.palette.primary)
                        radius: 8
                        border.width: !authButton.enabled ? 1 : 0
                        border.color: Theme.palette.borderInactive
                    }

                    contentItem: Text {
                        text: parent.text
                        color: authButton.enabled ? Theme.palette.bgMain : Theme.palette.textMuted
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        font.bold: true
                    }
                }
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: PolkitState.cancel()
    }
}
