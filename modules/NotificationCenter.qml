pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../globals"
import "../components"

PanelWindow {
    id: center

    property bool keyboardFocusRequested: false
    property bool panelClosing: false
    readonly property bool centerOpen: GlobalState.showNotificationCenter

    function requestKeyboardFocus(): void {
        keyboardFocusRequested = true;
    }

    function releaseKeyboardFocus(): void {
        keyboardFocusRequested = false;
    }

    anchors.top: true
    anchors.bottom: center.centerOpen
    anchors.right: true
    implicitWidth: 430
    implicitHeight: center.centerOpen ? 720 : (toastStack.implicitHeight + 32)

    color: "transparent"
    exclusiveZone: -1
    visible: center.centerOpen || center.activeToasts.length > 0 || center.panelClosing

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: center.keyboardFocusRequested ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

    onCenterOpenChanged: {
        if (center.centerOpen) {
            panelClosing = false;
            panelHideTimer.stop();
            return;
        }

        if (center.activeToasts.length === 0) {
            panelClosing = true;
            panelHideTimer.restart();
        }
    }

    onActiveToastsChanged: {
        if (center.centerOpen)
            return;

        if (center.activeToasts.length > 0) {
            panelClosing = false;
            panelHideTimer.stop();
        } else {
            panelClosing = true;
            panelHideTimer.restart();
        }
    }

    Timer {
        id: panelHideTimer
        interval: 250
        repeat: false
        running: false
        onTriggered: center.panelClosing = false
    }

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
            if (!!item.dismissed)
                continue;
            if (!!item.toastExpired)
                continue;
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

    ListView {
        id: toastStack
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 16
        anchors.rightMargin: 18
        width: 400
        height: contentHeight
        implicitHeight: contentHeight
        spacing: 10
        interactive: false
        clip: false
        model: center.activeToasts

        add: Transition {
            ParallelAnimation {
                NumberAnimation {
                    property: "x"
                    from: toastStack.width + 24
                    duration: 180
                    easing.type: Easing.OutCubic
                }
                NumberAnimation {
                    property: "opacity"
                    from: 0
                    to: 1
                    duration: 150
                    easing.type: Easing.OutQuad
                }
            }
        }

        remove: Transition {
            ParallelAnimation {
                NumberAnimation {
                    property: "x"
                    to: toastStack.width + 24
                    duration: 160
                    easing.type: Easing.InCubic
                }
                NumberAnimation {
                    property: "opacity"
                    to: 0
                    duration: 130
                    easing.type: Easing.InQuad
                }
            }
        }

        displaced: Transition {
            NumberAnimation {
                properties: "x,y"
                duration: 160
                easing.type: Easing.OutCubic
            }
        }

        HoverHandler {
            onHoveredChanged: {
                if (hovered)
                    center.requestKeyboardFocus();
                else
                    center.releaseKeyboardFocus();
            }
        }

        delegate: Item {
            id: notifItem
            required property var modelData
            width: toastStack.width
            height: toastItem.implicitHeight

            NotificationToast {
                id: toastItem
                anchors.right: parent.right
                notification: notifItem.modelData
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
        anchors.rightMargin: center.centerOpen ? 12 : -width - 24
        radius: 12
        color: Theme.palette.bgMain
        visible: center.centerOpen

        Timer {
            id: hideTimer
            interval: 220
            repeat: false
            onTriggered: panel.visible = false
        }

        Connections {
            target: center
            function onCenterOpenChanged() {
                if (center.centerOpen) {
                    hideTimer.stop();
                    panel.visible = true;
                } else {
                    hideTimer.restart();
                }
            }
        }

        HoverHandler {
            onHoveredChanged: {
                if (hovered)
                    center.requestKeyboardFocus();
                else
                    center.releaseKeyboardFocus();
            }
        }

        states: [
            State {
                name: "open"
                when: center.centerOpen
                PropertyChanges {
                    panel.anchors.rightMargin: 12
                }
            },
            State {
                name: "closed"
                when: !center.centerOpen
                PropertyChanges {
                    panel.anchors.rightMargin: -width - 24
                }
            }
        ]

        transitions: [
            Transition {
                from: "closed"
                to: "open"
                NumberAnimation {
                    target: panel
                    property: "anchors.rightMargin"
                    duration: 220
                    easing.type: Easing.OutCubic
                }
            }
        ]

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 10

            RowLayout {
                Layout.fillWidth: true

                Text {
                    text: "Notifications"
                    color: Theme.palette.textMain
                    font.pixelSize: 14
                    font.bold: true
                    font.family: Theme.palette.font
                    Layout.fillWidth: true
                }

                CompactIconButton {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    iconText: GlobalState.doNotDisturb ? "󰂛" : "󰂚"
                    iconColor: Theme.palette.textMain
                    backgroundColor: GlobalState.doNotDisturb ? Theme.palette.error : Theme.palette.bgWidget
                    hoverBackgroundColor: GlobalState.doNotDisturb ? Theme.palette.error : Theme.palette.bgHover
                    borderColor: GlobalState.doNotDisturb ? Theme.palette.error : Theme.palette.borderInactive
                    hoverBorderColor: GlobalState.doNotDisturb ? Theme.palette.error : Theme.palette.borderActive
                    onClicked: GlobalState.setDoNotDisturb(!GlobalState.doNotDisturb)
                }

                CompactIconButton {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    iconText: "󰄬"
                    iconColor: Theme.palette.textMain
                    onClicked: GlobalState.markAllRead()
                }

                CompactIconButton {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    iconText: "󰃢"
                    iconColor: Theme.palette.textMain
                    onClicked: GlobalState.clearAllNotifications()
                }

                CompactIconButton {
                    Layout.preferredWidth: 30
                    Layout.preferredHeight: 24
                    iconText: "󰅖"
                    iconColor: Theme.palette.textMain
                    onClicked: {
                        GlobalState.showNotificationCenter = false;
                        center.releaseKeyboardFocus();
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.palette.bgWidget
                radius: 10
                border.width: 1
                border.color: Theme.palette.borderInactive

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
