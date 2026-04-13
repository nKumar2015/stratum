pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../globals"

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
    visible: true

    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    property int selectedIndex: 0
    property var powerOptions: [
        // ... (lines 28-57)
    ]

    // ... (lines 60-68)

    Shortcut {
        sequence: "Escape"
        onActivated: {
            PowerState.showPowerMenu = false;
        }
    }

    // ... (lines 79-95)

    MouseArea {
        anchors.fill: parent
        onClicked: {
            PowerState.showPowerMenu = false;
        }
    }

    Rectangle {
        implicitWidth: menuLayout.implicitWidth + 40
        implicitHeight: menuLayout.implicitHeight + 40
        anchors.centerIn: parent

        color: Theme.palette.bgMain
        border.color: Theme.palette.borderActive
        border.width: 2
        radius: 12

        MouseArea {
            anchors.fill: parent
        }

        ColumnLayout {
            anchors.centerIn: parent
            spacing: 20

            RowLayout {
                id: menuLayout
                spacing: 10

                Repeater {
                    model: powerMenu.powerOptions
                    delegate: Rectangle {
                        id: powerOption
                        required property var modelData
                        required property int index

                        property bool isActive: powerMenu.selectedIndex === index

                        Layout.preferredWidth: 140
                        Layout.preferredHeight: 140
                        color: Theme.palette.bgWidget
                        border.color: isActive ? Theme.palette.borderActive : Theme.palette.borderInactive
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
                                    colorizationColor: powerOption.isActive ? Theme.palette.secondary : Theme.palette.bgDark
                                }
                            }
                            Text {
                                id: btnText
                                anchors.horizontalCenter: parent.horizontalCenter
                                text: powerOption.modelData.name
                                color: powerOption.isActive ? Theme.palette.textMain : Theme.palette.textMuted
                                font.pixelSize: 10
                                font.family: Theme.palette.font
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
                            }
                        }
                    }
                }
            }
        }
    }
}
