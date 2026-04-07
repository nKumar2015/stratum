pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"

PanelWindow {
    id: powerMenu

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    WlrLayershell.layer: WlrLayer.Overlay
    color: "#a0000000"
    visible: false

    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    property int selectedIndex: 0
    property int confirmIndex: -1
    property string confirmMessage: ""
    readonly property int confirmWindowMs: 5000
    property var powerOptions: [
        {
            name: "Shutdown",
            command: ["systemctl", "poweroff"],
            icon: "../theme/icons/shutdown.svg"
        },
        {
            name: "Reboot",
            command: ["systemctl", "reboot"],
            icon: "../theme/icons/reboot.svg"
        },
        {
            name: "Suspend",
            command: ["systemctl", "suspend"],
            icon: "../theme/icons/suspend.svg"
        },
        {
            name: "Logout",
            command: ["hyprctl", "dispatch", "exit"],
            icon: "../theme/icons/logout.svg"
        },
        {
            name: "Reboot into Windows",
            command: ["systemctl", "reboot", "--boot-loader-entry=windows.conf"],
            icon: "../theme/icons/windows.svg"
        },
        {
            name: "Reboot into BIOS",
            command: ["systemctl", "reboot", "--firmware-setup"],
            icon: "../theme/icons/bios.svg"
        }
    ]

    Process {
        id: cmdRunner
    }

    Timer {
        id: confirmTimer
        interval: powerMenu.confirmWindowMs
        repeat: false
        onTriggered: powerMenu.resetConfirmation()
    }

    function resetConfirmation() {
        confirmIndex = -1;
        confirmMessage = "";
        confirmTimer.stop();
    }

    function requiresConfirmation(index) {
        const actionName = String(powerOptions[index]?.name || "");
        return actionName === "Shutdown" || actionName === "Reboot" || actionName === "Reboot into Windows" || actionName === "Reboot into BIOS";
    }

    function executeSelected() {
        if (requiresConfirmation(selectedIndex) && confirmIndex !== selectedIndex) {
            confirmIndex = selectedIndex;
            confirmMessage = "Press Enter again to confirm " + powerOptions[selectedIndex].name;
            confirmTimer.restart();
            return;
        }

        resetConfirmation();
        powerMenu.visible = false;
        cmdRunner.command = powerOptions[selectedIndex].command;
        cmdRunner.running = true;
    }

    IpcHandler {
        target: "powermenu"
        function toggle(): void {
            powerMenu.visible = !powerMenu.visible;
            if (!powerMenu.visible)
                powerMenu.resetConfirmation();
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            powerMenu.visible = false;
            powerMenu.resetConfirmation();
        }
    }

    Shortcut {
        sequence: "Left"
        onActivated: {
            powerMenu.selectedIndex = (powerMenu.selectedIndex > 0) ? powerMenu.selectedIndex - 1 : powerMenu.powerOptions.length - 1;
            powerMenu.resetConfirmation();
        }
    }

    Shortcut {
        sequence: "Right"
        onActivated: {
            powerMenu.selectedIndex = (powerMenu.selectedIndex < powerMenu.powerOptions.length - 1) ? powerMenu.selectedIndex + 1 : 0;
            powerMenu.resetConfirmation();
        }
    }

    Shortcut {
        sequence: "Return"
        onActivated: powerMenu.executeSelected()
    }

    MouseArea {
        anchors.fill: parent
        onClicked: {
            powerMenu.visible = false;
            powerMenu.resetConfirmation();
        }
    }

    Rectangle {
        implicitWidth: menuLayout.implicitWidth + 40
        implicitHeight: menuLayout.implicitHeight + 40
        anchors.centerIn: parent

        color: Theme.background
        border.color: Theme.surfaceContainerLow
        border.width: 2
        radius: 12

        MouseArea {
            anchors.fill: parent
        }

        RowLayout {
            id: menuLayout
            anchors.centerIn: parent
            anchors.margins: 20
            spacing: 20

            Repeater {

                model: powerMenu.powerOptions
                delegate: Rectangle {
                    id: powerOption
                    required property var modelData
                    required property int index

                    property bool isActive: powerMenu.selectedIndex === index

                    Layout.preferredWidth: 140
                    Layout.preferredHeight: 140
                    color: Theme.background
                    border.color: isActive ? Theme.primary : Theme.background
                    border.width: 1
                    radius: 8

                    Behavior on color {
                        ColorAnimation {
                            duration: 150
                        }
                    }

                    Column {
                        anchors.centerIn: parent
                        spacing: 16
                        Item {
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: 100
                            height: 100
                            Image {
                                id: btnIcon
                                anchors.fill: parent
                                source: powerOption.modelData.icon
                                fillMode: Image.PreserveAspectFit

                                sourceSize.width: 100
                                sourceSize.height: 100
                                visible: false
                            }
                            MultiEffect {
                                source: btnIcon
                                anchors.fill: btnIcon
                                colorization: 1.0
                                brightness: 1
                                colorizationColor: powerOption.isActive ? Theme.on_Surface : Theme.surfaceContainer
                            }
                        }
                        Text {
                            id: btnText
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: powerOption.modelData.name
                            color: powerOption.isActive ? Theme.on_Surface : Theme.surfaceContainer
                            font.pixelSize: 10
                            font.family: Theme.font
                            font.bold: true
                        }
                    }
                    MouseArea {
                        id: btnMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: {
                            powerMenu.selectedIndex = powerOption.index;
                            powerMenu.executeSelected();
                        }
                        onEntered: {
                            powerMenu.selectedIndex = powerOption.index;
                            powerMenu.resetConfirmation();
                        }
                    }
                }
            }
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 12
            visible: powerMenu.confirmMessage.length > 0
            text: powerMenu.confirmMessage
            color: Theme.error
            font.pixelSize: 11
            font.family: Theme.font
        }
    }
}
