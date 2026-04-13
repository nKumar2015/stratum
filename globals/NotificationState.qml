pragma Singleton
import QtQuick

QtObject {
    property bool showCenter: false
    property bool doNotDisturb: false
    property int maxNotifications: 50
    property int nextNotificationId: 1
    property var notifications: []

    signal actionRequested(int notificationId, string actionKey, string replyText)
    signal dismissRequested(int notificationId, bool expired)

    function normalizeProgress(value) {
        const parsed = Number(value);
        if (isNaN(parsed)) return -1;
        if (parsed < 0) return -1;
        return Math.max(0, Math.min(100, Math.round(parsed)));
    }

    function normalizeActions(actions) {
        if (!actions || typeof actions.length !== "number") return [];
        const out = [];
        for (let i = 0; i < actions.length; i++) {
            const action = actions[i] || {};
            const id = String(action.id || action.key || "").trim();
            const label = String(action.label || action.text || "").trim();
            if (!id || !label) continue;
            out.push({ id: id, label: label, inlineReply: !!action.inlineReply });
        }
        return out;
    }

    function normalizeExpiryMs(value) {
        const parsed = Number(value);
        if (isNaN(parsed) || parsed < 0) return 5000;
        if (parsed === 0) return 0;
        return Math.max(1, Math.round(parsed));
    }

    function normalizeNotificationEntry(data, fallbackId) {
        const source = data || {};
        const urgency = Math.max(0, Math.min(2, parseInt(source.urgency || 1)));
        const id = Number(source.id || fallbackId || nextNotificationId);
        const ts = Number(source.timestamp || Date.now());
        const updated = Number(source.updatedAt || ts);

        return {
            id: id,
            appId: String(source.appId || ""),
            appName: String(source.appName || "Unknown"),
            summary: String(source.summary || "Notification"),
            body: String(source.body || ""),
            urgency: urgency,
            actions: normalizeActions(source.actions || []),
            timestamp: ts,
            updatedAt: updated,
            expiryMs: normalizeExpiryMs(source.expiryMs),
            read: !!source.read,
            dismissed: !!source.dismissed,
            toastExpired: !!source.toastExpired,
            defaultActionKey: String(source.defaultActionKey || "default"),
            inlineReplyActionKey: String(source.inlineReplyActionKey || ""),
            inlineReplyPlaceholder: String(source.inlineReplyPlaceholder || "Reply..."),
            sourceNotificationId: Number(source.sourceNotificationId || 0),
            replacesId: Number(source.replacesId || 0),
            category: String(source.category || ""),
            progressValue: normalizeProgress(source.progressValue),
            progressSourceHint: String(source.progressSourceHint || ""),
            imageUrl: String(source.imageUrl || ""),
            appIcon: String(source.appIcon || ""),
            closeReason: String(source.closeReason || ""),
            lastInlineReplyText: String(source.lastInlineReplyText || "")
        };
    }

    function addNotification(payload) {
        const id = nextNotificationId++;
        const entry = normalizeNotificationEntry(payload, id);
        notifications = [entry].concat(notifications).slice(0, maxNotifications);
        return id;
    }

    function patchNotification(notificationId, patch) {
        const idx = notifications.findIndex(n => n.id === notificationId);
        if (idx < 0) return;
        const clone = notifications.slice();
        const next = Object.assign({}, clone[idx], patch);
        // re-normalize some fields if they were patched
        if (patch.actions) next.actions = normalizeActions(next.actions);
        if (patch.progressValue !== undefined) next.progressValue = normalizeProgress(next.progressValue);
        clone[idx] = next;
        notifications = clone;
    }

    function dismissNotification(notificationId) {
        patchNotification(notificationId, { closeReason: "Dismissed", dismissed: true, read: true });
        dismissRequested(notificationId, false);
    }

    function expireToast(notificationId) {
        patchNotification(notificationId, { closeReason: "Expired", toastExpired: true });
        dismissRequested(notificationId, true);
    }

    function markRead(notificationId) {
        patchNotification(notificationId, { read: true });
    }

    function markAllRead() {
        notifications = notifications.map(n => Object.assign({}, n, { read: true }));
    }

    function clearAllNotifications() {
        notifications = [];
    }

    function findNotificationIdBySource(sourceNotificationId) {
        const sourceId = Number(sourceNotificationId || 0);
        if (sourceId <= 0) return -1;
        for (let i = 0; i < notifications.length; i++) {
            if (Number(notifications[i].sourceNotificationId || 0) === sourceId)
                return Number(notifications[i].id || -1);
        }
        return -1;
    }

    function normalizeSnapshotNotifications(list) {
        if (!list || typeof list.length !== "number") return [];
        const out = [];
        for (let i = 0; i < list.length; i++) {
            out.push(normalizeNotificationEntry(list[i], Number(list[i].id || i + 1)));
        }
        return out;
    }

    function invokeAction(notificationId, actionKey, replyText) {
        const key = String(actionKey || "").trim();
        if (!key) return;
        if (replyText) patchNotification(notificationId, { lastInlineReplyText: replyText, updatedAt: Date.now() });
        markRead(notificationId);
        actionRequested(notificationId, key, replyText || "");
    }

    function invokeDefaultAction(notificationId) {
        const idx = notifications.findIndex(n => n.id === notificationId);
        if (idx < 0) return;
        const defaultKey = String(notifications[idx].defaultActionKey || "default");
        invokeAction(notificationId, defaultKey, "");
    }

    function setDoNotDisturb(enabled) {
        doNotDisturb = !!enabled;
    }
}
