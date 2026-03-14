import QtQuick
import QtQuick.Layouts
import Quickshell.Widgets

import "../theme"

Item {
    id: root

    property var notification: ({})
    property bool compact: false
    property bool autoDismissEnabled: true
    property bool showDismissedState: false
    property bool showReplyPreviewOnly: false

    signal dismissRequested(int notificationId)
    signal expiredRequested(int notificationId)
    signal actionRequested(int notificationId, string actionKey, string replyText)
    signal defaultActionRequested(int notificationId)

    readonly property int urgency: Number(notification.urgency || 1)
    readonly property bool sticky: urgency >= 2 || Number(notification.expiryMs || 5000) <= 0
    readonly property bool hovered: toastHover.hovered
    readonly property bool dismissed: !!notification.dismissed
    readonly property int progressValue: Number(notification.progressValue)
    readonly property string progressSourceHint: String(notification.progressSourceHint || "")
    readonly property bool hasProgress: progressSourceHint.length > 0 && !isNaN(progressValue) && progressValue >= 0
    readonly property string imageSource: String(notification.imageUrl || "")
    readonly property string appIconSource: String(notification.appIcon || "")
    readonly property bool hasImageSource: imageSource.length > 0
    readonly property bool hasAppIconSource: appIconSource.length > 0
    readonly property bool visualLoading: hasImageSource && visualImage.status === Image.Loading
    readonly property bool visualReady: (hasImageSource && visualImage.status === Image.Ready) || (!hasImageSource && hasAppIconSource)
    readonly property string iconGlyph: urgency >= 2 ? "" : (urgency <= 0 ? "" : "")
    readonly property color accentColor: urgency >= 2 ? Theme.notificationCritical : (urgency <= 0 ? Theme.notificationLow : Theme.notificationNormal)
    readonly property string inlineReplyPreviewText: String(notification.lastInlineReplyText || "")
    readonly property var interactiveActions: {
        const actions = notification.actions || [];
        const out = [];
        for (let i = 0; i < actions.length; i++) {
            const action = actions[i] || {};
            const id = String(action.id || "");
            if (!id || id === "default" || !!action.inlineReply)
                continue;
            out.push(action);
        }
        return out;
    }
    readonly property bool hasDefaultAction: {
        const actions = notification.actions || [];
        for (let i = 0; i < actions.length; i++) {
            if (String(actions[i].id || "") === String(notification.defaultActionKey || "default"))
                return true;
        }
        return Number(notification.sourceNotificationId || 0) > 0;
    }

    function findInlineReplyAction() {
        const actions = notification.actions || [];
        const preferred = String(notification.inlineReplyActionKey || "");
        if (preferred) {
            for (let i = 0; i < actions.length; i++) {
                if (String(actions[i].id || "") === preferred)
                    return actions[i];
            }
        }
        for (let i = 0; i < actions.length; i++) {
            if (!!actions[i].inlineReply)
                return actions[i];
        }
        return null;
    }

    implicitWidth: compact ? 360 : 400
    implicitHeight: Math.max(88, contentCol.implicitHeight + 20)

    HoverHandler {
        id: toastHover
    }

    Rectangle {
        id: bg
        anchors.fill: parent
        topLeftRadius: 0
        topRightRadius: 8
        bottomLeftRadius: 0
        bottomRightRadius: 8
        color: Theme.background
        border.width: 1
        border.color: root.hovered ? root.accentColor : (root.showDismissedState && root.dismissed ? Theme.grey : Theme.notificationBorder)
        opacity: root.showDismissedState && root.dismissed ? 0.72 : 1.0

        Behavior on border.color {
            ColorAnimation {
                duration: 120
            }
        }
    }

    Rectangle {
        width: 4
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.margins: 0
        radius: 2
        color: root.accentColor
    }

    MouseArea {
        id: openDefault
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        propagateComposedEvents: true
        enabled: root.hasDefaultAction
        onClicked: {
            mouse.accepted = false;
            root.defaultActionRequested(Number(notification.id || 0));
        }
    }

    ColumnLayout {
        id: contentCol
        anchors.fill: parent
        anchors.margins: 10
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            spacing: 8

            Rectangle {
                Layout.preferredWidth: 20
                Layout.preferredHeight: 20
                radius: 5
                color: "transparent"
                border.width: 0
                visible: root.visualReady || root.visualLoading

                Image {
                    id: visualImage
                    anchors.fill: parent
                    anchors.margins: 1
                    source: root.imageSource
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    asynchronous: true
                    visible: root.hasImageSource && visualImage.status === Image.Ready
                }

                IconImage {
                    anchors.fill: parent
                    anchors.margins: 1
                    source: root.appIconSource
                    visible: !root.hasImageSource && root.hasAppIconSource
                }
            }

            Text {
                text: root.iconGlyph
                color: root.accentColor
                font.pixelSize: 16
                font.family: Theme.font
                visible: !root.visualReady
            }

            Text {
                Layout.fillWidth: true
                text: String(notification.summary || "Notification")
                color: Theme.text
                elide: Text.ElideRight
                font.pixelSize: 12
                font.bold: true
                font.family: Theme.font
            }

            Text {
                text: String(notification.appName || "")
                color: Theme.notificationMuted
                font.pixelSize: 10
                font.family: Theme.font
                visible: text.length > 0
            }

            Rectangle {
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
                radius: 9
                color: "transparent"
                border.width: 1
                border.color: Theme.grey

                Text {
                    anchors.centerIn: parent
                    text: "✕"
                    color: Theme.text
                    font.pixelSize: 10
                    font.family: Theme.font
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: root.dismissRequested(Number(notification.id || 0))
                }
            }
        }

        Text {
            Layout.fillWidth: true
            text: String(notification.body || "")
            color: Theme.white
            wrapMode: Text.Wrap
            maximumLineCount: compact ? 3 : 6
            elide: Text.ElideRight
            font.pixelSize: 11
            font.family: Theme.font
            visible: text.length > 0
        }

        Text {
            Layout.fillWidth: true
            text: String(notification.category || "")
            color: Theme.notificationMuted
            elide: Text.ElideRight
            font.pixelSize: 10
            font.family: Theme.font
            visible: text.length > 0
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            visible: root.hasProgress

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 6
                radius: 3
                color: Theme.black
                border.width: 1
                border.color: Theme.grey

                Rectangle {
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    anchors.left: parent.left
                    width: Math.max(0, Math.min(parent.width, parent.width * (root.progressValue / 100.0)))
                    radius: 3
                    color: root.accentColor
                }
            }

            Text {
                Layout.alignment: Qt.AlignRight
                text: root.progressValue + "%"
                color: Theme.notificationMuted
                font.pixelSize: 10
                font.family: Theme.font
                visible: !root.compact
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            visible: root.interactiveActions.length > 0

            Repeater {
                model: root.interactiveActions
                delegate: Rectangle {
                    required property var modelData

                    Layout.preferredHeight: 24
                    Layout.preferredWidth: Math.max(74, actionText.implicitWidth + 18)
                    radius: 7
                    color: "transparent"
                    border.width: 1
                    border.color: root.accentColor

                    Text {
                        id: actionText
                        anchors.centerIn: parent
                        text: String(modelData.label || "Action")
                        color: root.accentColor
                        font.pixelSize: 10
                        font.bold: true
                        font.family: Theme.font
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: root.actionRequested(Number(notification.id || 0), String(modelData.id || ""), "")
                    }
                }
            }
        }

        RowLayout {
            id: inlineReplyRow
            Layout.fillWidth: true
            spacing: 6

            property var replyAction: root.findInlineReplyAction()
            visible: !!replyAction && !root.showReplyPreviewOnly

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.black
                border.width: 1
                border.color: Theme.grey

                TextInput {
                    id: replyInput
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    color: Theme.text
                    selectionColor: Theme.activeWs
                    selectedTextColor: Theme.black
                    font.pixelSize: 11
                    font.family: Theme.font
                    clip: true

                    property string placeholderText: String(notification.inlineReplyPlaceholder || "Reply...")

                    onAccepted: {
                        const text = replyInput.text.trim();
                        if (!text)
                            return;
                        const actionId = String(inlineReplyRow.replyAction.id || "");
                        root.actionRequested(Number(notification.id || 0), actionId, text);
                        replyInput.text = "";
                    }
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 8
                    text: replyInput.placeholderText
                    color: Theme.notificationMuted
                    visible: replyInput.text.length === 0 && !replyInput.activeFocus
                    font.pixelSize: 10
                    font.family: Theme.font
                }
            }

            Rectangle {
                Layout.preferredWidth: 52
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.black
                border.width: 1
                border.color: root.accentColor

                Text {
                    anchors.centerIn: parent
                    text: "Send"
                    color: root.accentColor
                    font.pixelSize: 10
                    font.bold: true
                    font.family: Theme.font
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        const text = replyInput.text.trim();
                        if (!text)
                            return;
                        const actionId = String(inlineReplyRow.replyAction.id || "");
                        root.actionRequested(Number(notification.id || 0), actionId, text);
                        replyInput.text = "";
                    }
                }
            }
        }

        RowLayout {
            id: inlineReplyPreviewRow
            Layout.fillWidth: true
            spacing: 6

            property var replyAction: root.findInlineReplyAction()
            visible: !!replyAction && root.showReplyPreviewOnly

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.black
                border.width: 1
                border.color: Theme.grey

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 6

                    Text {
                        text: "Reply:"
                        color: Theme.notificationMuted
                        font.pixelSize: 10
                        font.family: Theme.font
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.inlineReplyPreviewText.length > 0 ? root.inlineReplyPreviewText : "(no reply sent)"
                        color: Theme.text
                        elide: Text.ElideRight
                        wrapMode: Text.NoWrap
                        clip: true
                        font.pixelSize: 10
                        font.family: Theme.font
                    }
                }
            }
        }
    }

    Timer {
        id: dismissTimer
        interval: Math.max(1200, Number(notification.expiryMs || 5000))
        repeat: false
        running: root.autoDismissEnabled && !root.sticky && !root.hovered && !replyInput.activeFocus
        onTriggered: root.expiredRequested(Number(notification.id || 0))
    }
}
