pragma Singleton
import QtQuick

QtObject {
    property string popupMonitorName: ""
    property bool screenshotOverlayOpen: false
    property bool showWifiSettings: false
    property bool showWifiHoverMenu: false
    property real wifiIconY: 0
    property bool wifiHoverIntent: false
    property string wifiState: "disconnected"
    property int wifiSignalPercent: 0
    property bool wifiEthernet: false
    property bool wifiEnabled: false
    property bool showAudioHoverMenu: false
    property real audioIconY: 0
    property bool audioHoverIntent: false
    property int audioVolumePercent: 0
    property bool audioMuted: true
    property bool audioHeadphonesOutput: false
    property bool audioUserAdjusting: false
    property bool daemonAvailable: false
    property bool showAudioMenu: false
    property string musicTitle: ""
    property string musicArtist: ""
    property string musicAlbum: ""
    property string musicPlayer: ""
    property string musicStatus: ""
    property int musicPosition: 0
    property int musicLength: 0
    property string musicArtUrl: ""
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
    property bool doNotDisturb: false
    property int maxNotifications: 50
    property int nextNotificationId: 1
    property var notifications: []

    signal notificationActionRequested(int notificationId, string actionKey, string replyText)
    signal notificationDismissRequested(int notificationId, bool expired)
    signal screenshotViewerOpenRequested(string imagePath, string captureMode)

    function setPopupMonitorName(name) {
        popupMonitorName = String(name || "");
    }

    function parseDaemonPayload(payloadText) {
        const text = String(payloadText || "").trim();
        if (!text.length)
            return null;

        try {
            return JSON.parse(text);
        } catch (_error) {
            return null;
        }
    }

    function applyDaemonAudioSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        const audio = (payload && payload.audio && typeof payload.audio === "object") ? payload.audio : payload;
        if (!audio || typeof audio !== "object")
            return;

        const parsedVolume = parseInt(String(audio.volume || "0").replace("%", ""));
        audioVolumePercent = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(150, parsedVolume));
        audioMuted = String(audio.mute || "yes").trim().toLowerCase() === "yes";
        audioHeadphonesOutput = String(audio.headphones || "no").trim().toLowerCase() === "yes";
        daemonAvailable = true;
    }

    function applyDaemonWifiSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        const wifi = (payload && payload.net && typeof payload.net === "object") ? payload.net : payload;
        if (!wifi || typeof wifi !== "object")
            return;

        const state = String(wifi.state || "disconnected").trim().toLowerCase();
        wifiState = state;
        wifiEthernet = state === "ethernet";
        wifiEnabled = state === "ethernet" || state === "wifi";

        const signal = parseInt(String(wifi.signal_pct || "0"));
        wifiSignalPercent = isNaN(signal) ? 0 : Math.max(0, Math.min(100, signal));
        daemonAvailable = true;
    }

    function applyDaemonBluetoothSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        const bluetooth = (payload && payload.bluetooth && typeof payload.bluetooth === "object") ? payload.bluetooth : payload;
        if (!bluetooth || typeof bluetooth !== "object")
            return;

        const raw = String(bluetooth.state || "off").trim().toLowerCase();
        bluetoothPowered = raw === "connected" || raw === "on";
        bluetoothConnected = raw === "connected";
        bluetoothScanning = String(bluetooth.scanning || "no").trim().toLowerCase() === "yes";
        daemonAvailable = true;
    }

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
        for (let i = 0; i < list.length; i++) {
            out.push(normalizeNotificationEntry(list[i], Number(list[i].id || i + 1)));
        }
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
