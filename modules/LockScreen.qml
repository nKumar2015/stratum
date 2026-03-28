pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Pam
import Quickshell.Io
import QtQuick.Effects
import Quickshell.Widgets

import "../theme"

Scope {
    id: lockRoot

    property bool authFailed: false
    property string timeString: new Date().toLocaleTimeString(Qt.locale(), "hh:mm AP")
    function updateTime() {
        let now = new Date();
        timeString = now.toLocaleTimeString(Qt.locale(), "hh:mm AP");

        timer.interval = (60 - now.getSeconds()) * 1000 - now.getMilliseconds();
        timer.restart();
    }

    Timer {
        id: timer
        interval: 1000 * 60
        repeat: true
        running: true
        onTriggered: lockRoot.updateTime()
    }

    IpcHandler {
        target: "lockscreen"
        function lock(): void {
            lockScreen.locked = true;
            lockRoot.authFailed = false;
            if (!pam.active) {
                pam.start();
            }
        }
    }

    PamContext {
        id: pam
        config: "quickshell"

        onCompleted: result => {
            console.log("PAM Completed with result code: " + result);
            if (result === PamResult.Success) {
                lockScreen.locked = false;
                lockRoot.authFailed = false;
            } else {
                lockRoot.authFailed = true;
                pam.start();
            }
        }
    }

    WlSessionLock {
        id: lockScreen
        locked: false

        WlSessionLockSurface {
            Item {
                anchors.fill: parent
                Image {
                    id: bgImage
                    anchors.fill: parent
                    source: "file:///home/nakul/Pictures/Wallpapers/mountain.jpg"
                    fillMode: Image.PreserveAspectCrop
                    visible: true
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        blurEnabled: true
                        blurMax: 30
                        blur: 0.8
                    }
                }

                Rectangle {
                    anchors.fill: parent
                    color: "#8011111b"
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: passwordInput.forceActiveFocus()
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    spacing: 24

                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: lockRoot.timeString
                        color: Theme.primary // Using your active workspace color for emphasis
                        font {
                            pixelSize: 50
                            bold: true
                            family: Theme.font // Or your preferred monospace font
                        }
                    }

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.preferredWidth: 120
                        Layout.preferredHeight: 120
                        radius: 60
                        color: "transparent"
                        border.width: 3
                        border.color: Theme.primary
                        ClippingWrapperRectangle {
                            width: 120
                            height: 120
                            radius: width / 2
                            anchors.fill: parent
                            anchors.margins: 3
                            Image {
                                id: avatar
                                anchors.fill: parent
                                anchors.margins: -1
                                source: "file:///home/nakul/Pictures/pfp.png"

                                visible: true
                            }
                        }
                    }

                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: "Enter Password"
                        color: Theme.on_Surface
                        font.pixelSize: 20
                        font.bold: true
                        font.family: Theme.font
                    }

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.preferredWidth: 300
                        Layout.preferredHeight: 50
                        color: Theme.surfaceCon_tainerLowest
                        radius: 8
                        border.color: passwordInput.activeFocus ? Theme.primary : Theme.surfaceCon_tainer
                        border.width: 2

                        TextInput {
                            id: passwordInput
                            anchors.fill: parent
                            anchors.margins: 14
                            verticalAlignment: TextInput.AlignVCenter
                            color: Theme.on_Surface
                            font.pixelSize: 18
                            echoMode: TextInput.Password
                            focus: true

                            Connections {
                                target: lockScreen
                                function onLockedChanged() {
                                    if (lockScreen.locked) {
                                        passwordInput.forceActiveFocus();
                                        passwordInput.text = "";
                                    }
                                }
                            }

                            onAccepted: {
                                console.log("Enter key pressed! responseRequired is: " + pam.responseRequired);
                                lockRoot.authFailed = false;
                                if (pam.responseRequired) {
                                    pam.respond(passwordInput.text);
                                    passwordInput.text = "";
                                }
                            }
                        }
                    }

                    Text {
                        id: errorText
                        Layout.alignment: Qt.AlignHCenter
                        text: "Incorrect password, try again."
                        color: Theme.error
                        font.pixelSize: 14
                        visible: lockRoot.authFailed
                        font.family: Theme.font
                    }
                }
            }
        }
    }
}
