pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io
import Qt5Compat.GraphicalEffects

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
    property var powerOptions: [
        {
            name: "Shutdown",
            cmd: "systemctl poweroff",
            icon: "../theme/icons/shutdown.svg"
        },
        {
            name: "Reboot",
            cmd: "systemctl reboot",
            icon: "../theme/icons/reboot.svg"
        },
        {
            name: "Suspend",
            cmd: "systemctl suspend",
            icon: "../theme/icons/suspend.svg"
        },
        {
            name: "Logout",
            cmd: "hyprctl dispatch exit",
            icon: "../theme/icons/logout.svg"
        },
        {
            name: "Reboot into Windows",
            cmd: "systemctl reboot --boot-loader-entry=windows.conf",
            icon: "../theme/icons/windows.svg"
        },
        {
            name: "Reboot into BIOS",
            cmd: "systemctl reboot --firmware-setup",
            icon: "../theme/icons/bios.svg"
        }
    ]

    Process {
        id: cmdRunner
    }

    function executeSelected() {
        powerMenu.visible = false;
        cmdRunner.command = ["bash", "-c", powerOptions[selectedIndex].cmd];
        cmdRunner.running = true;
    }

    IpcHandler {
        target: "powermenu"
        function toggle(): void {
            powerMenu.visible = !powerMenu.visible;
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: powerMenu.visible = false
    }

    Shortcut {
        sequence: "Left"
        onActivated: powerMenu.selectedIndex = (powerMenu.selectedIndex > 0) ? powerMenu.selectedIndex - 1 : powerMenu.powerOptions.length - 1
    }

    Shortcut {
        sequence: "Right"
        onActivated: powerMenu.selectedIndex = (powerMenu.selectedIndex < powerMenu.powerOptions.length - 1) ? powerMenu.selectedIndex + 1 : 0
    }

    Shortcut {
        sequence: "Return"
        onActivated: powerMenu.executeSelected()
    }

    MouseArea {
        anchors.fill: parent
        onClicked: powerMenu.visible = false
    }

    Rectangle {
        implicitWidth: menuLayout.implicitWidth + 40
        implicitHeight: menuLayout.implicitHeight + 40
        anchors.centerIn: parent

        color: Theme.background
        border.color: Theme.inactiveWs
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
                    required property var modelData
                    required property int index

                    property bool isActive: powerMenu.selectedIndex === index

                    Layout.preferredWidth: 140
                    Layout.preferredHeight: 140
                    color: Theme.background
                    border.color: isActive ? Theme.activeWs : Theme.background
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
                                Layout.alignment: Qt.AlignHCenter
                                Layout.preferredWidth: 100
                                Layout.preferredHeight: 100

                                source: modelData.icon
                                fillMode: Image.PreserveAspectFit

                                sourceSize.width: 100
                                sourceSize.height: 100
                            }

                            ColorOverlay {
                                anchors.fill: btnIcon
                                source: btnIcon
                                color: isActive ? Theme.text : Theme.hover
                            }
                        }
                        Text {
                            id: btnText
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: modelData.name
                            color: isActive ? Theme.text : Theme.hover
                            font.pixelSize: 10
                            font.bold: true
                        }
                    }
                    MouseArea {
                        id: btnMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: powerMenu.executeSelected()
                        onEntered: powerMenu.selectedIndex = index
                    }
                }
            }
        }
    }
}
