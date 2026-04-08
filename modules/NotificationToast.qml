pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import Quickshell.Widgets

import "../globals"

Item {
    id: root

    property var notification: ({})
    property bool compact: false
    property bool autoDismissEnabled: true
    property bool showDismissedState: false
    property bool showReplyPreviewOnly: false
    property bool animatePopupTransitions: compact

    signal dismissRequested(int notificationId)
    signal expiredRequested(int notificationId)
    signal actionRequested(int notificationId, string actionKey, string replyText)
    signal defaultActionRequested(int notificationId)

    readonly property int urgency: Number(notification.urgency || 1)
    readonly property int effectiveExpiryMs: {
        const parsed = Number(notification.expiryMs);
        if (isNaN(parsed) || parsed < 0)
            return 5000;
        if (parsed === 0)
            return 0;
        return Math.max(1, Math.round(parsed));
    }
    readonly property bool sticky: urgency >= 2 || root.effectiveExpiryMs === 0
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
    readonly property color accentColor: urgency >= 2 ? Theme.palette.error : (urgency <= 0 ? Theme.palette.tertiary : Theme.palette.primary)
    readonly property color accentContainerColor: urgency >= 2 ? Theme.palette.error : (urgency <= 0 ? Theme.palette.success : Theme.palette.warning)
    readonly property color onAccentContainerColor: Theme.palette.textMain
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
    readonly property bool canAutoDismiss: root.autoDismissEnabled && !root.sticky && !root.hovered && !replyInput.activeFocus
    property string pendingCloseKind: ""
    property bool closeAnimationRunning: false

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
    transformOrigin: Item.TopRight

    function requestDismissWithAnimation() {
        dismissTimer.stop();
        const id = Number(notification.id || 0);
        if (!root.animatePopupTransitions || root.closeAnimationRunning) {
            root.dismissRequested(id);
            return;
        }

        pendingCloseKind = "dismiss";
        closeAnimationRunning = true;
        closeAnim.start();
    }

    function requestExpireWithAnimation() {
        dismissTimer.stop();
        const id = Number(notification.id || 0);
        if (!root.animatePopupTransitions || root.closeAnimationRunning) {
            root.expiredRequested(id);
            return;
        }

        pendingCloseKind = "expire";
        closeAnimationRunning = true;
        closeAnim.start();
    }

    ParallelAnimation {
        id: openAnim
        NumberAnimation {
            target: root
            property: "opacity"
            from: 0
            to: 1
            duration: 150
            easing.type: Easing.OutQuad
        }
        NumberAnimation {
            target: root
            property: "scale"
            from: 0.97
            to: 1
            duration: 180
            easing.type: Easing.OutCubic
        }
    }

    ParallelAnimation {
        id: closeAnim
        NumberAnimation {
            target: root
            property: "opacity"
            to: 0
            duration: 130
            easing.type: Easing.InQuad
        }
        NumberAnimation {
            target: root
            property: "scale"
            to: 0.96
            duration: 150
            easing.type: Easing.InCubic
        }
        onStopped: {
            const id = Number(notification.id || 0);
            const reason = pendingCloseKind;
            pendingCloseKind = "";
            closeAnimationRunning = false;

            if (reason === "expire")
                root.expiredRequested(id);
            else
                root.dismissRequested(id);
        }
    }

    HoverHandler {
        id: toastHover
    }

    Rectangle {
        id: bg
        anchors.fill: parent
        topLeftRadius: 0
        topRightRadius: 12
        bottomLeftRadius: 0
        bottomRightRadius: 12
        color: Theme.palette.bgMain
        border.width: 1
        border.color: root.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
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
                color: Theme.palette.bgMain
                border.width: 1
                border.color: Theme.palette.borderInactive
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
                font.family: Theme.palette.font
                visible: !root.visualReady
            }

            Text {
                Layout.fillWidth: true
                text: String(notification.summary || "Notification")
                color: Theme.palette.textMain
                elide: Text.ElideRight
                font.pixelSize: 12
                font.bold: true
                font.family: Theme.palette.font
            }

            Text {
                text: String(notification.appName || "")
                color: Theme.palette.textMain
                font.pixelSize: 10
                font.family: Theme.palette.font
                visible: text.length > 0
            }

            Rectangle {
                Layout.preferredWidth: 18
                Layout.preferredHeight: 18
                radius: 9
                color: dismissHover.containsMouse ? Theme.palette.bgHover : Theme.palette.bgMain
                border.width: 1
                border.color: Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "✕"
                    color: Theme.palette.error
                    font.pixelSize: 10
                    font.family: Theme.palette.font
                }

                MouseArea {
                    id: dismissHover
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: root.requestDismissWithAnimation()
                }
            }
        }

        Text {
            Layout.fillWidth: true
            text: String(notification.body || "")
            color: Theme.palette.textMain
            wrapMode: Text.Wrap
            maximumLineCount: compact ? 3 : 6
            elide: Text.ElideRight
            font.pixelSize: 11
            font.family: Theme.palette.font
            visible: text.length > 0
        }

        Text {
            Layout.fillWidth: true
            text: String(notification.category || "")
            color: Theme.palette.textMain
            elide: Text.ElideRight
            font.pixelSize: 10
            font.family: Theme.palette.font
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
                color: Theme.palette.bgMain
                border.width: 1
                border.color: Theme.palette.borderInactive

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
                color: Theme.palette.textMain
                font.pixelSize: 10
                font.family: Theme.palette.font
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
                    color: root.accentContainerColor
                    border.width: 0

                    Text {
                        id: actionText
                        anchors.centerIn: parent
                        text: String(modelData.label || "Action")
                        color: root.onAccentContainerColor
                        font.pixelSize: 10
                        font.bold: true
                        font.family: Theme.palette.font
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
                color: Theme.palette.bgMain
                border.width: 1
                border.color: Theme.palette.borderInactive

                TextInput {
                    id: replyInput
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    verticalAlignment: TextInput.AlignVCenter
                    color: Theme.palette.bgWidget
                    selectionColor: Theme.palette.primary
                    selectedTextColor: Theme.palette.textMain
                    font.pixelSize: 11
                    font.family: Theme.palette.font
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
                    color: Theme.palette.textMuted
                    visible: replyInput.text.length === 0 && !replyInput.activeFocus
                    font.pixelSize: 10
                    font.family: Theme.palette.font
                }
            }

            Rectangle {
                Layout.preferredWidth: 52
                Layout.preferredHeight: 28
                radius: 6
                color: root.accentContainerColor
                border.width: 0

                Text {
                    anchors.centerIn: parent
                    text: "Send"
                    color: root.onAccentContainerColor
                    font.pixelSize: 10
                    font.bold: true
                    font.family: Theme.palette.font
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
                color: Theme.palette.bgMain
                border.width: 1
                border.color: Theme.palette.borderInactive

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 6

                    Text {
                        text: "Reply:"
                        color: Theme.palette.textMain
                        font.pixelSize: 10
                        font.family: Theme.palette.font
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.inlineReplyPreviewText.length > 0 ? root.inlineReplyPreviewText : "(no reply sent)"
                        color: Theme.palette.textMain
                        elide: Text.ElideRight
                        wrapMode: Text.NoWrap
                        clip: true
                        font.pixelSize: 10
                        font.family: Theme.palette.font
                    }
                }
            }
        }
    }

    Timer {
        id: dismissTimer
        interval: Math.max(1200, root.effectiveExpiryMs)
        repeat: false
        running: false
        onTriggered: root.requestExpireWithAnimation()
    }

    function syncDismissTimer() {
        dismissTimer.stop();
        if (root.canAutoDismiss)
            dismissTimer.start();
    }

    onCanAutoDismissChanged: syncDismissTimer()
    onNotificationChanged: syncDismissTimer()
    Component.onCompleted: {
        syncDismissTimer();
        if (root.animatePopupTransitions) {
            root.opacity = 0;
            root.scale = 0.97;
            openAnim.start();
        }
    }
}
