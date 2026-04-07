import QtQuick

QtObject {
    property bool doNotDisturb: false
    property int maxNotifications: 50
    property int nextNotificationId: 1
    property var notifications: []

    function normalizeProgress(value) {
        const parsed = Number(value);
        if (isNaN(parsed))
            return -1;
        if (parsed < 0)
            return -1;
        return Math.max(0, Math.min(100, Math.round(parsed)));
    }

    function normalizeActions(actions) {
        if (!actions || typeof actions.length !== "number")
            return [];

        const out = [];
        for (let i = 0; i < actions.length; i++) {
            const action = actions[i] || {};
            const id = String(action.id || action.key || "").trim();
            const label = String(action.label || action.text || "").trim();
            if (!id || !label)
                continue;

            out.push({
                id: id,
                label: label,
                inlineReply: !!action.inlineReply
            });
        }
        return out;
    }

    function normalizeExpiryMs(value) {
        const parsed = Number(value);
        if (isNaN(parsed) || parsed < 0)
            return 5000;
        if (parsed === 0)
            return 0;
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

    function normalizeSnapshotNotifications(list) {
        if (!list || typeof list.length !== "number")
            return [];

        const out = [];
        for (let i = 0; i < list.length; i++)
            out.push(normalizeNotificationEntry(list[i], Number(list[i].id || i + 1)));
        return out;
    }

    function trimHistory() {
        if (notifications.length <= maxNotifications)
            return;

        notifications = notifications.slice(0, maxNotifications);
    }

    function addNotification(payload) {
        const data = payload || {};
        const urgency = Math.max(0, Math.min(2, parseInt(data.urgency || 1)));
        if (doNotDisturb && urgency < 2)
            return -1;

        const id = nextNotificationId;
        nextNotificationId = nextNotificationId + 1;

        const entry = normalizeNotificationEntry({
            id: id,
            appId: data.appId,
            appName: data.appName,
            summary: data.summary,
            body: data.body,
            urgency: urgency,
            actions: data.actions,
            timestamp: data.timestamp || Date.now(),
            updatedAt: data.updatedAt || Date.now(),
            expiryMs: data.expiryMs,
            read: false,
            dismissed: false,
            toastExpired: false,
            defaultActionKey: data.defaultActionKey,
            inlineReplyActionKey: data.inlineReplyActionKey,
            inlineReplyPlaceholder: data.inlineReplyPlaceholder,
            sourceNotificationId: data.sourceNotificationId,
            replacesId: data.replacesId,
            category: data.category,
            progressValue: data.progressValue,
            progressSourceHint: data.progressSourceHint,
            imageUrl: data.imageUrl,
            appIcon: data.appIcon,
            closeReason: data.closeReason,
            lastInlineReplyText: data.lastInlineReplyText
        }, id);

        notifications = [entry].concat(notifications);
        trimHistory();
        return id;
    }

    function findIndexById(notificationId) {
        for (let i = 0; i < notifications.length; i++) {
            if (notifications[i].id === notificationId)
                return i;
        }
        return -1;
    }

    function findNotificationIdBySource(sourceNotificationId) {
        const sourceId = Number(sourceNotificationId || 0);
        if (sourceId <= 0)
            return -1;

        for (let i = 0; i < notifications.length; i++) {
            if (Number(notifications[i].sourceNotificationId || 0) === sourceId)
                return Number(notifications[i].id || -1);
        }
        return -1;
    }

    function cloneNotificationEntry(entry) {
        const source = entry || {};
        const next = {};
        for (const key in source)
            next[key] = source[key];
        return next;
    }

    function mergeNotificationPatch(current, patch) {
        const next = cloneNotificationEntry(current);
        const update = patch || {};
        for (const patchKey in update)
            next[patchKey] = update[patchKey];

        next.actions = normalizeActions(next.actions || []);
        next.progressValue = normalizeProgress(next.progressValue);
        next.expiryMs = normalizeExpiryMs(next.expiryMs);
        next.updatedAt = Number(next.updatedAt || Date.now());
        return next;
    }

    function patchNotification(notificationId, patch) {
        const idx = findIndexById(notificationId);
        if (idx < 0)
            return;

        const clone = notifications.slice();
        clone[idx] = mergeNotificationPatch(clone[idx], patch);
        notifications = clone;
    }

    function markRead(notificationId) {
        patchNotification(notificationId, {
            read: true
        });
    }

    function markAllRead() {
        const clone = notifications.slice();
        for (let i = 0; i < clone.length; i++) {
            clone[i] = mergeNotificationPatch(clone[i], {
                read: true
            });
        }
        notifications = clone;
    }

    function clearAllNotifications() {
        notifications = [];
    }
}
