pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import Quickshell
import Quickshell.Widgets
import Quickshell.Services.SystemTray

import "../globals"

Flow {
    id: trayRoot
    required property var panelWindow

    spacing: 6
    QsMenuOpener {
        id: menuOpener
    }

    PopupWindow {
        id: customMenu
        visible: false
        color: "transparent"

        implicitWidth: menuBackground.width
        implicitHeight: menuBackground.height

        property bool sourceIconHovered: false

        function checkHoverStatus() {
            closeTimer.restart();
        }

        Timer {
            id: closeTimer
            interval: 150
            onTriggered: {
                if (!menuHover.hovered && !customMenu.sourceIconHovered) {
                    customMenu.visible = false;
                }
            }
        }

        onVisibleChanged: {
            if (!visible)
                menuOpener.menu = null;
        }

        Rectangle {
            id: menuBackground

            HoverHandler {
                id: menuHover
                onHoveredChanged: {
                    if (!hovered && customMenu.visible) {
                        customMenu.checkHoverStatus();
                    }
                }
            }

            // Added Math.max so the popup never collapses to 0x0 while waiting for DBus
            implicitWidth: Math.max(120, menuLayout.implicitWidth + 8)
            implicitHeight: Math.max(32, menuLayout.implicitHeight + 8)
            color: Theme.palette.bgMain
            border.color: Theme.palette.borderActive
            border.width: 1
            radius: 6

            ColumnLayout {
                id: menuLayout
                anchors.centerIn: parent
                spacing: 2

                Repeater {
                    model: menuOpener.children

                    delegate: Rectangle {
                        id: trayItem
                        required property var modelData

                        implicitWidth: Math.max(150, entryText.implicitWidth + 24)
                        implicitHeight: modelData.isSeparator ? 1 : 28
                        color: entryMouse.containsMouse ? Theme.palette.bgHover : "transparent"
                        radius: 4

                        Text {
                            id: entryText
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.left: parent.left
                            anchors.leftMargin: 8
                            text: trayItem.modelData.isSeparator ? "" : trayItem.modelData.text.replace(/&/g, "")
                            color: Theme.palette.textMain
                            font.pixelSize: 13
                            font.family: Theme.palette.font
                            visible: !trayItem.modelData.isSeparator
                        }

                        Rectangle {
                            anchors.fill: parent
                            anchors.leftMargin: 4
                            anchors.rightMargin: 4
                            color: Theme.palette.secondary
                            visible: trayItem.modelData.isSeparator
                        }

                        MouseArea {
                            id: entryMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            enabled: !trayItem.modelData.isSeparator && trayItem.modelData.enabled

                            onClicked: {
                                trayItem.modelData.triggered();
                                customMenu.visible = false;
                            }
                        }
                    }
                }
            }
        }
    }
    ColumnLayout {
        Repeater {
            model: SystemTray.items

            delegate: Rectangle {
                id: trayIconWrap
                required property var modelData

                implicitWidth: 24
                implicitHeight: 24
                color: mouseArea.containsMouse ? "#313244" : "transparent"
                radius: 4

                // Replaced QtQuick Image with Quickshell IconImage
                IconImage {
                    anchors.centerIn: parent
                    implicitWidth: 18
                    implicitHeight: 18
                    source: trayIconWrap.modelData.icon ? trayIconWrap.modelData.icon : Quickshell.iconPath(trayIconWrap.modelData.id, "application-x-executable")
                }

                MouseArea {
                    id: mouseArea
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton

                    onContainsMouseChanged: {
                        if (menuOpener.menu === trayIconWrap.modelData.menu) {
                            customMenu.sourceIconHovered = containsMouse;

                            if (!containsMouse && customMenu.visible) {
                                customMenu.checkHoverStatus();
                            }
                        }
                    }

                    onClicked: mouse => {
                        if (mouse.button === Qt.LeftButton) {
                            trayIconWrap.modelData.activate();
                        } else if (mouse.button === Qt.RightButton) {
                            if (trayIconWrap.modelData.hasMenu) {
                                menuOpener.menu = trayIconWrap.modelData.menu;
                                customMenu.sourceIconHovered = true;
                                let parentWindow = trayRoot.panelWindow;

                                if (parentWindow) {
                                    customMenu.anchor.window = parentWindow;
                                    let baseRect = parentWindow.itemRect(trayIconWrap);
                                    let shiftX = 10;
                                    customMenu.anchor.rect = Qt.rect(baseRect.x + shiftX, baseRect.y, baseRect.width, baseRect.height);
                                    customMenu.anchor.edges = Edges.Right;
                                    customMenu.anchor.gravity = Edges.Right;
                                    customMenu.visible = true;
                                } else {
                                    console.warn("Failed to find Window. Check your imports or pass the Sidebar ID directly.");
                                }
                            } else {
                                // FALLBACK: App doesn't use DBus menus, but expects a right-click signal!
                                trayIconWrap.modelData.secondaryActivate();
                            }
                        }
                    }
                }
            }
        }
    }
}
