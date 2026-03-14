import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: center

    property bool keyboardFocusRequested: false

    function requestKeyboardFocus(): void {
        keyboardFocusRequested = true;
    }

    function releaseKeyboardFocus(): void {
        keyboardFocusRequested = false;
    }

    anchors.top: true
    anchors.bottom: true
    anchors.right: true
    implicitWidth: 430

    color: "transparent"
    exclusiveZone: -1
    visible: GlobalState.showNotificationCenter || center.activeToasts.length > 0

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: center.keyboardFocusRequested ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

    onVisibleChanged: {
        if (!visible)
            center.releaseKeyboardFocus();
    }

    readonly property var activeToasts: {
        const out = [];
        const list = GlobalState.notifications || [];
        for (let i = 0; i < list.length; i++) {
            const item = list[i];
            if (!!item.dismissed)
                continue;
            if (!!item.toastExpired)
                continue;
            out.push(item);
            if (out.length >= 4)
                break;
        }
        return out;
    }

    readonly property var visibleNotifications: {
        const out = [];
        const list = GlobalState.notifications || [];
        for (let i = 0; i < list.length; i++) {
            const item = list[i];
            out.push(item);
        }
        return out;
    }

    IpcHandler {
        target: "notifications"

        function open(): void {
            GlobalState.showNotificationCenter = true;
        }

        function close(): void {
            GlobalState.showNotificationCenter = false;
            center.releaseKeyboardFocus();
        }

        function toggle(): void {
            GlobalState.showNotificationCenter = !GlobalState.showNotificationCenter;
        }

        function clear(): void {
            GlobalState.clearAllNotifications();
        }

        function toggleDnd(): void {
            GlobalState.setDoNotDisturb(!GlobalState.doNotDisturb);
        }

        // Keep IPC target strictly typed by arity; dynamic-typed arguments trigger
        // unsupported QVariant warnings in current quickshell IPC parser.
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (GlobalState.showNotificationCenter)
                GlobalState.showNotificationCenter = false;
        }
    }

    Column {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 16
        anchors.rightMargin: 18
        spacing: 10

        HoverHandler {
            onHoveredChanged: {
                if (hovered)
                    center.requestKeyboardFocus();
                else
                    center.releaseKeyboardFocus();
            }
        }

        Repeater {
            model: center.activeToasts
            delegate: NotificationToast {
                required property var modelData
                notification: modelData
                compact: true
                autoDismissEnabled: true
                onDismissRequested: notificationId => GlobalState.dismissNotification(notificationId)
                onExpiredRequested: notificationId => GlobalState.expireToast(notificationId)
                onActionRequested: (notificationId, actionKey, replyText) => {
                    GlobalState.invokeAction(notificationId, actionKey, replyText);
                    GlobalState.dismissNotification(notificationId);
                }
                onDefaultActionRequested: notificationId => {
                    GlobalState.invokeDefaultAction(notificationId);
                    GlobalState.dismissNotification(notificationId);
                }
            }
        }
    }

    Rectangle {
        id: panel
        width: 420
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        anchors.topMargin: 12
        anchors.bottomMargin: 12
        anchors.rightMargin: GlobalState.showNotificationCenter ? 12 : -width - 24
        radius: 12
        color: Theme.background
        border.width: 1
        border.color: Theme.notificationBorder
        visible: GlobalState.showNotificationCenter

        HoverHandler {
            onHoveredChanged: {
                if (hovered)
                    center.requestKeyboardFocus();
                else
                    center.releaseKeyboardFocus();
            }
        }

        Behavior on anchors.rightMargin {
            NumberAnimation {
                duration: 180
                easing.type: Easing.OutCubic
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 10

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "Notifications"
                    color: Theme.text
                    font.pixelSize: 14
                    font.bold: true
                    font.family: Theme.font
                    Layout.fillWidth: true
                }

                Rectangle {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    radius: 6
                    color: GlobalState.doNotDisturb ? Theme.notificationCritical : Theme.black
                    border.width: 1
                    border.color: GlobalState.doNotDisturb ? Theme.notificationCritical : Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: GlobalState.doNotDisturb ? "󰂛" : "󰂚"
                        color: Theme.text
                        font.pixelSize: 13
                        font.bold: true
                        font.family: Theme.font
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: GlobalState.setDoNotDisturb(!GlobalState.doNotDisturb)
                    }
                }

                Rectangle {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    radius: 6
                    color: Theme.black
                    border.width: 1
                    border.color: Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: "󰄬"
                        color: Theme.white
                        font.pixelSize: 13
                        font.bold: true
                        font.family: Theme.font
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: GlobalState.markAllRead()
                    }
                }

                Rectangle {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    radius: 6
                    color: Theme.black
                    border.width: 1
                    border.color: Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: "󰃢"
                        color: Theme.white
                        font.pixelSize: 13
                        font.bold: true
                        font.family: Theme.font
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: GlobalState.clearAllNotifications()
                    }
                }

                Rectangle {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    radius: 6
                    color: Theme.black
                    border.width: 1
                    border.color: Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.white
                        font.pixelSize: 13
                        font.bold: true
                        font.family: Theme.font
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            GlobalState.showNotificationCenter = false;
                            center.releaseKeyboardFocus();
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.background
                radius: 10
                border.width: 1
                border.color: Theme.grey

                Flickable {
                    id: listFlick
                    anchors.fill: parent
                    anchors.margins: 8
                    contentWidth: width
                    contentHeight: historyCol.implicitHeight
                    clip: true

                    Column {
                        id: historyCol
                        width: listFlick.width
                        spacing: 8

                        Repeater {
                            model: center.visibleNotifications
                            delegate: NotificationToast {
                                required property var modelData
                                width: historyCol.width
                                notification: modelData
                                compact: false
                                autoDismissEnabled: false
                                showDismissedState: true
                                showReplyPreviewOnly: true
                                onDismissRequested: notificationId => GlobalState.dismissNotification(notificationId)
                                onExpiredRequested: notificationId => GlobalState.expireToast(notificationId)
                                onActionRequested: (notificationId, actionKey, replyText) => GlobalState.invokeAction(notificationId, actionKey, replyText)
                                onDefaultActionRequested: notificationId => GlobalState.invokeDefaultAction(notificationId)
                            }
                        }
                    }
                }
            }
        }
    }
}
