pragma Singleton
import QtQuick

QtObject {
    property string popupMonitorName: ""
    property bool screenshotOverlayOpen: false
    property bool showWifiSettings: false
    property bool showWifiHoverMenu: false
    property real wifiIconY: 0
    property bool wifiHoverIntent: false
    property bool showAudioHoverMenu: false
    property real audioIconY: 0
    property bool audioHoverIntent: false
    property int audioVolumePercent: 0
    property bool audioMuted: true
    property bool audioUserAdjusting: false
    property bool showBluetoothSettings: false
    property bool showBluetoothHoverMenu: false
    property real bluetoothIconY: 0
    property bool bluetoothHoverIntent: false
    property bool bluetoothPowered: false
    property bool bluetoothConnected: false
    property bool bluetoothScanning: false
    property bool showBatteryHoverMenu: false
    property real batteryIconY: 0
    property bool batteryHoverIntent: false
    property bool showDashboardMenu: false

    property bool showNotificationCenter: false

    NotificationStore {
        id: notificationStore
    }

    property alias doNotDisturb: notificationStore.doNotDisturb
    property alias maxNotifications: notificationStore.maxNotifications
    property alias nextNotificationId: notificationStore.nextNotificationId
    property alias notifications: notificationStore.notifications

    signal notificationActionRequested(int notificationId, string actionKey, string replyText)
    signal notificationDismissRequested(int notificationId, bool expired)
    signal screenshotViewerOpenRequested(string imagePath, string captureMode)

    function setPopupMonitorName(name) {
        popupMonitorName = String(name || "");
    }

    function normalizeProgress(value) {
        return notificationStore.normalizeProgress(value);
    }

    function normalizeActions(actions) {
        return notificationStore.normalizeActions(actions);
    }

    function normalizeNotificationEntry(data, fallbackId) {
        return notificationStore.normalizeNotificationEntry(data, fallbackId);
    }

    function normalizeSnapshotNotifications(list) {
        return notificationStore.normalizeSnapshotNotifications(list);
    }

    function trimHistory() {
        notificationStore.trimHistory();
    }

    function addNotification(payload) {
        return notificationStore.addNotification(payload);
    }

    function findIndexById(notificationId) {
        return notificationStore.findIndexById(notificationId);
    }

    function findNotificationIdBySource(sourceNotificationId) {
        return notificationStore.findNotificationIdBySource(sourceNotificationId);
    }

    function cloneNotificationEntry(entry) {
        return notificationStore.cloneNotificationEntry(entry);
    }

    function mergeNotificationPatch(current, patch) {
        return notificationStore.mergeNotificationPatch(current, patch);
    }

    function patchNotification(notificationId, patch) {
        notificationStore.patchNotification(notificationId, patch);
    }

    function dismissNotification(notificationId) {
        patchNotification(notificationId, {
            closeReason: "Dismissed",
            dismissed: true,
            read: true
        });
        notificationDismissRequested(notificationId, false);
    }

    function expireToast(notificationId) {
        patchNotification(notificationId, {
            closeReason: "Expired",
            toastExpired: true
        });
        notificationDismissRequested(notificationId, true);
    }

    function markRead(notificationId) {
        notificationStore.markRead(notificationId);
    }

    function markAllRead() {
        notificationStore.markAllRead();
    }

    function clearAllNotifications() {
        notificationStore.clearAllNotifications();
    }

    function setDoNotDisturb(enabled) {
        doNotDisturb = !!enabled;
    }

    function invokeAction(notificationId, actionKey, replyText) {
        const key = String(actionKey || "").trim();
        if (!key)
            return;

        const reply = String(replyText || "").trim();
        if (reply.length > 0) {
            patchNotification(notificationId, {
                lastInlineReplyText: reply,
                updatedAt: Date.now()
            });
        }

        markRead(notificationId);
        notificationActionRequested(notificationId, key, reply);
    }

    function invokeDefaultAction(notificationId) {
        const idx = findIndexById(notificationId);
        if (idx < 0)
            return;

        const defaultKey = String(notifications[idx].defaultActionKey || "default");
        invokeAction(notificationId, defaultKey, "");
    }
}
