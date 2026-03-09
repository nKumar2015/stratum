pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Services.Pam
import Quickshell.Io

Scope {
    id: lockRoot

    property bool authFailed: false

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
            Rectangle {
                anchors.fill: parent
                color: "#1e1e2e"

                MouseArea {
                    anchors.fill: parent
                    onClicked: passwordInput.forceActiveFocus()
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    spacing: 24

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        width: 120
                        height: 120
                        radius: 60
                        color: "#313244"
                        border.color: "#89b4fa"
                        border.width: 3
                        Text {
                            anchors.centerIn: parent
                            text: "󰣇"
                            color: "#89b4fa"
                            font.pixelSize: 48
                        }
                    }

                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        text: pam.message || "Enter Password"
                        color: "#cdd6f4"
                        font.pixelSize: 20
                        font.bold: true
                    }

                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        width: 300
                        height: 50
                        color: "#11111b"
                        radius: 8
                        border.color: passwordInput.activeFocus ? "#89b4fa" : "#45475a"
                        border.width: 2

                        TextInput {
                            id: passwordInput
                            anchors.fill: parent
                            anchors.margins: 14
                            verticalAlignment: TextInput.AlignVCenter
                            color: "#cdd6f4"
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
                        color: "#f38ba8"
                        font.pixelSize: 14
                        visible: lockRoot.authFailed
                    }
                }
            }
        }
    }
}
