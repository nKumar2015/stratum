import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Notifications

import "../globals"

Scope {
    id: root

    property bool snapshotLoaded: false
    property bool restoringSnapshot: false
    property var localToNative: ({})
    property var localToActions: ({})
    readonly property int snapshotVersion: 1

    function shallowCopyMap(map) {
        const out = {};
        const source = map || {};
        for (const key in source)
            out[key] = source[key];
        return out;
    }

    function mapWithEntry(map, key, value) {
        const next = shallowCopyMap(map);
        next[String(key)] = value;
        return next;
    }

    function mapWithoutKey(map, key) {
        const targetKey = String(key);
        const source = map || {};
        const next = {};
        for (const mapKey in source) {
            if (mapKey === targetKey)
                continue;
            next[mapKey] = source[mapKey];
        }
        return next;
    }

    function mapFilteredByKeys(map, allowedKeys) {
        const allowed = allowedKeys || {};
        const source = map || {};
        const next = {};
        for (const key in source) {
            if (!!allowed[key])
                next[key] = source[key];
        }
        return next;
    }

    function nativeNotificationFor(localId) {
        return (root.localToNative || {})[String(localId)];
    }

    function nativeActionsFor(localId) {
        return (root.localToActions || {})[String(localId)] || {};
    }

    function buildSnapshotPayload() {
        return {
            version: snapshotVersion,
            doNotDisturb: !!NotificationState.doNotDisturb,
            nextNotificationId: Number(NotificationState.nextNotificationId || 1),
            notifications: NotificationState.notifications || []
        };
    }

    function normalizeSnapshotNotification(entry) {
        const current = entry || {};
        const next = {};
        for (const key in current)
            next[key] = current[key];

        next.imageUrl = root.toImageSource(current.imageUrl);
        next.appIcon = root.toStringValue(current.appIcon);
        return next;
    }

    function nextNotificationIdForSnapshot(parsedNextId, notifications) {
        const list = notifications || [];
        let maxId = 0;
        for (let i = 0; i < list.length; i++)
            maxId = Math.max(maxId, Number(list[i].id || 0));
        return Math.max(maxId + 1, Number(parsedNextId || (list.length + 1)), 1);
    }

    function queueSnapshotSave() {
        if (!snapshotLoaded || restoringSnapshot)
            return;
        snapshotSaveTimer.restart();
    }

    function saveSnapshotNow() {
        if (restoringSnapshot)
            return;

        const payload = buildSnapshotPayload();
        const json = JSON.stringify(payload);
        const encoded = encodeURIComponent(json);
        snapshotSaveProc.command = ["stratum-cli", "notifications-snapshot", "snapshot-save", encoded];
        snapshotSaveProc.running = true;
    }

    function applySnapshot(raw) {
        const text = (raw || "").trim();
        if (!text)
            return false;

        try {
            const parsed = JSON.parse(text);
            if (!parsed || typeof parsed !== "object")
                return false;

            const list = parsed.notifications;
            if (!list || typeof list.length !== "number")
                return false;

            restoringSnapshot = true;
            const normalized = NotificationState.normalizeSnapshotNotifications(list);
            for (let i = 0; i < normalized.length; i++)
                normalized[i] = root.normalizeSnapshotNotification(normalized[i]);

            NotificationState.doNotDisturb = !!parsed.doNotDisturb;
            NotificationState.notifications = normalized.slice(0, Math.max(1, Number(NotificationState.maxNotifications || 50)));

            NotificationState.nextNotificationId = root.nextNotificationIdForSnapshot(parsed.nextNotificationId, NotificationState.notifications);
            restoringSnapshot = false;
            return true;
        } catch (err) {
            restoringSnapshot = false;
            console.log("notifications: failed to parse snapshot", err);
            return false;
        }
    }

    function normalizeUrgency(urgency) {
        const value = Number(urgency);
        if (isNaN(value))
            return 1;
        return Math.max(0, Math.min(2, value));
    }

    function hintValue(notification, keys) {
        const hints = notification.hints || {};
        for (let i = 0; i < keys.length; i++) {
            const value = hints[keys[i]];
            if (value !== undefined && value !== null)
                return value;
        }
        return undefined;
    }

    function toStringValue(value) {
        if (value === undefined || value === null)
            return "";
        return String(value).trim();
    }

    function canonicalFileUrl(pathOrUrl) {
        const text = root.toStringValue(pathOrUrl);
        if (text.length === 0)
            return "";

        if (text.startsWith("file://")) {
            const withoutScheme = text.substring("file://".length);
            const normalizedPath = "/" + withoutScheme.replace(/^\/+/, "");
            return "file://" + normalizedPath;
        }

        const normalizedPath = "/" + text.replace(/^\/+/, "");
        return "file://" + normalizedPath;
    }

    function toImageSource(value) {
        let text = root.toStringValue(value);
        if (text.length === 0)
            return "";

        while (text.startsWith("image://icon/image://icon/")) {
            text = "image://icon/" + text.substring("image://icon/image://icon/".length);
        }

        if (text.startsWith("image://icon//")) {
            const rawPath = text.substring("image://icon//".length);
            return root.canonicalFileUrl(rawPath);
        }

        if (text.startsWith("file://"))
            return root.canonicalFileUrl(text);

        if (text.startsWith("qrc:/") || text.startsWith("data:") || text.startsWith("http://") || text.startsWith("https://") || text.startsWith("image://"))
            return text;

        if (text.startsWith("/"))
            return root.canonicalFileUrl(text);

        const resolved = root.toStringValue(Quickshell.iconPath(text));
        if (resolved.length > 0) {
            if (resolved.startsWith("/"))
                return root.canonicalFileUrl(resolved);
            return resolved;
        }

        return "";
    }

    function extractCategory(notification) {
        const direct = root.toStringValue(notification.category);
        if (direct.length > 0)
            return direct;

        const value = root.hintValue(notification, [
            "category",
            "x-kde-notification-category",
            "desktop-entry-category"
        ]);
        return root.toStringValue(value);
    }

    function syncNativeMaps() {
        const validIds = {};
        const list = NotificationState.notifications || [];
        for (let i = 0; i < list.length; i++) {
            const id = Number(list[i].id || 0);
            if (id > 0)
                validIds[String(id)] = true;
        }

        root.localToNative = root.mapFilteredByKeys(root.localToNative, validIds);
        root.localToActions = root.mapFilteredByKeys(root.localToActions, validIds);
    }

    function extractProgress(notification) {
        const keys = [
            "value",
            "x-kde-progress-value",
            "x-canonical-private-synchronous-progress"
        ];

        for (let i = 0; i < keys.length; i++) {
            const raw = root.hintValue(notification, [keys[i]]);
            if (raw === undefined || raw === null)
                continue;
            const parsed = Number(raw);
            if (isNaN(parsed))
                continue;
            return {
                value: Math.max(0, Math.min(100, Math.round(parsed))),
                source: keys[i]
            };
        }

        return {
            value: -1,
            source: ""
        };
    }

    function resolveImageUrl(notification) {
        const candidates = [
            notification.image,
            notification.imagePath,
            root.hintValue(notification, ["image-path"]),
            root.hintValue(notification, ["image_path"]),
            root.hintValue(notification, ["image-url"]),
            root.hintValue(notification, ["image_url"]),
            root.hintValue(notification, ["iCondata"])
        ];

        for (let i = 0; i < candidates.length; i++) {
            const source = root.toImageSource(candidates[i]);
            if (source.length > 0)
                return source;
        }
        return "";
    }

    function resolveAppIcon(notification) {
        const direct = [
            root.toStringValue(notification.appIcon),
            root.toStringValue(notification.icon),
            root.toStringValue(notification.iconName),
            root.toStringValue(root.hintValue(notification, ["app-icon", "icon"]))
        ];

        for (let i = 0; i < direct.length; i++) {
            if (direct[i].length > 0)
                return direct[i];
        }

        const appId = root.toStringValue(notification.desktopEntry);
        if (appId.length > 0) {
            const appResolved = root.toStringValue(Quickshell.iconPath(appId));
            if (appResolved.length > 0)
                return appResolved;
        }

        return "";
    }

    function resolveReplacesId(notification) {
        const direct = Number(notification.replacesId || 0);
        if (!isNaN(direct) && direct > 0)
            return direct;

        const hinted = Number(root.hintValue(notification, ["replaces-id", "x-kde-replaces-id", "id"]) || 0);
        if (isNaN(hinted) || hinted <= 0)
            return 0;
        return hinted;
    }

    function findBySourceId(sourceId) {
        const normalized = Number(sourceId || 0);
        if (isNaN(normalized) || normalized <= 0)
            return -1;

        const match = Number(NotificationState.findNotificationIdBySource(normalized));
        return match > 0 ? match : -1;
    }

    function normalizeIdentity(appId, appName, summary) {
        return {
            appId: root.toStringValue(appId),
            appName: root.toStringValue(appName),
            summary: root.toStringValue(summary)
        };
    }

    function notificationMatchesIdentity(item, identity) {
        if (root.toStringValue(item.summary) !== identity.summary)
            return false;
        if (identity.appName.length > 0 && root.toStringValue(item.appName) !== identity.appName)
            return false;
        if (identity.appId.length > 0 && root.toStringValue(item.appId).length > 0 && root.toStringValue(item.appId) !== identity.appId)
            return false;
        return true;
    }

    function findBySummaryIdentity(identity) {
        if (identity.summary.length === 0)
            return -1;

        const list = NotificationState.notifications || [];
        let fallbackAny = -1;
        for (let i = 0; i < list.length; i++) {
            const item = list[i] || {};
            if (!root.notificationMatchesIdentity(item, identity))
                continue;

            const id = Number(item.id || -1);
            if (id <= 0)
                continue;
            if (!item.dismissed && !item.toastExpired)
                return id;
            if (fallbackAny < 0)
                fallbackAny = id;
        }

        return fallbackAny;
    }

    function findLocalTargetId(notification, appId, appName, summary) {
        const sourceMatch = root.findBySourceId(notification.id);
        if (sourceMatch > 0)
            return sourceMatch;

        const replaceMatch = root.findBySourceId(root.resolveReplacesId(notification));
        if (replaceMatch > 0)
            return replaceMatch;

        return root.findBySummaryIdentity(root.normalizeIdentity(appId, appName, summary));
    }

    function inlineReplyHint(notification) {
        const hints = notification.hints || {};
        const keys = [
            "x-kde-reply-placeholder-text",
            "x-kde-reply-placeholder",
            "x-kde-inline-reply-placeholder-text"
        ];

        for (let i = 0; i < keys.length; i++) {
            const value = hints[keys[i]];
            if (value === undefined || value === null)
                continue;
            const text = String(value).trim();
            if (text.length > 0)
                return text;
        }

        return "";
    }

    function toActionModel(notification, inlineReplyEnabled) {
        const out = [];
        const nativeActions = notification.actions || [];
        for (let i = 0; i < nativeActions.length; i++) {
            const action = nativeActions[i];
            if (!action)
                continue;
            out.push({
                id: String(action.identifier || ""),
                label: String(action.text || "Action"),
                inlineReply: false
            });
        }

        if (inlineReplyEnabled) {
            out.push({
                id: "__inline_reply__",
                label: "Reply",
                inlineReply: true
            });
        }

        return out;
    }

    function registerNative(localId, notification) {
        root.localToNative = root.mapWithEntry(root.localToNative, localId, notification);

        const actionMap = {};
        const nativeActions = notification.actions || [];
        for (let i = 0; i < nativeActions.length; i++) {
            const action = nativeActions[i];
            if (!action)
                continue;
            actionMap[String(action.identifier || "")] = action;
        }

        root.localToActions = root.mapWithEntry(root.localToActions, localId, actionMap);

        notification.closed.connect(function(reason) {
            const reasonText = String(NotificationCloseReason.toString(reason));
            NotificationState.patchNotification(localId, {
                closeReason: reasonText,
                updatedAt: Date.now()
            });
            if (reasonText === "Expired") {
                NotificationState.expireToast(localId);
            } else {
                NotificationState.dismissNotification(localId);
            }
        });
    }

    function clearNative(localId) {
        root.localToNative = root.mapWithoutKey(root.localToNative, localId);
        root.localToActions = root.mapWithoutKey(root.localToActions, localId);
    }

    function onNativeNotification(notification) {
        const placeholderHint = root.inlineReplyHint(notification);
        const inlineReplyEnabled = !!notification.hasInlineReply || placeholderHint.length > 0;
        const placeholder = placeholderHint.length > 0 ? placeholderHint : String(notification.inlineReplyPlaceholder || "Reply...");
        const appId = String(notification.desktopEntry || "");
        const appName = String(notification.appName || "External");
        const summary = String(notification.summary || "Notification");
        const progress = root.extractProgress(notification);
        const category = root.extractCategory(notification);
        const replacesId = root.resolveReplacesId(notification);
        const existingLocalId = root.findLocalTargetId(notification, appId, appName, summary);

        const updatePayload = {
            appId: appId,
            appName: appName,
            summary: summary,
            body: String(notification.body || ""),
            urgency: root.normalizeUrgency(notification.urgency),
            actions: root.toActionModel(notification, inlineReplyEnabled),
            defaultActionKey: "default",
            inlineReplyActionKey: inlineReplyEnabled ? "__inline_reply__" : "",
            inlineReplyPlaceholder: placeholder,
            sourceNotificationId: Number(notification.id || 0),
            replacesId: replacesId,
            category: category,
            progressValue: progress.value,
            progressSourceHint: progress.source,
            imageUrl: root.resolveImageUrl(notification),
            appIcon: root.resolveAppIcon(notification),
            expiryMs: Number(notification.expireTimeout || 5000),
            closeReason: "",
            timestamp: Date.now(),
            updatedAt: Date.now(),
            dismissed: false,
            toastExpired: false,
            read: false
        };

        if (existingLocalId > 0) {
            NotificationState.patchNotification(existingLocalId, updatePayload);
            root.clearNative(existingLocalId);
            root.registerNative(existingLocalId, notification);
            return;
        }

        const localId = NotificationState.addNotification(updatePayload);

        if (localId < 0)
            return;

        root.registerNative(localId, notification);
    }

    function handleActionRequest(notificationId, actionKey, replyText) {
        const action = String(actionKey || "");
        const text = String(replyText || "");

        const notification = root.nativeNotificationFor(notificationId);
        if (!notification)
            return;

        if (action === "__inline_reply__") {
            if (text.length > 0)
                notification.sendInlineReply(text);
            return;
        }

        const actions = root.nativeActionsFor(notificationId);
        const target = actions[action];
        if (target)
            target.invoke();
    }

    function handleDismissRequest(notificationId, expired) {
        const notification = root.nativeNotificationFor(notificationId);
        if (!notification)
            return;

        if (!!expired)
            notification.expire();
        else
            notification.dismiss();
    }

    NotificationServer {
        id: notificationServer
        keepOnReload: true
        persistenceSupported: true
        bodySupported: true
        bodyMarkupSupported: true
        bodyHyperlinksSupported: true
        bodyImagesSupported: true
        actionsSupported: true
        actionIconsSupported: true
        imageSupported: true
        inlineReplySupported: true
        onNotification: notification => root.onNativeNotification(notification)
    }

    Process {
        id: snapshotSaveProc
    }

    Timer {
        id: snapshotSaveTimer
        interval: 700
        repeat: false
        onTriggered: root.saveSnapshotNow()
    }

    Process {
        id: snapshotLoadProc
        command: ["stratum-cli", "notifications-snapshot", "snapshot-load"]
        stdout: StdioCollector {
            onStreamFinished: {
                root.snapshotLoaded = root.applySnapshot(this.text);
                if (root.snapshotLoaded)
                    root.queueSnapshotSave();
            }
        }
    }

    Connections {
        target: NotificationState
        function onActionRequested(notificationId, actionKey, replyText) {
            root.handleActionRequest(notificationId, actionKey, replyText);
        }
        function onDismissRequested(notificationId, expired) {
            root.handleDismissRequest(notificationId, expired);
            root.clearNative(notificationId);
        }
        function onNotificationsChanged() {
            root.syncNativeMaps();
            root.queueSnapshotSave();
        }
        function onDoNotDisturbChanged() {
            root.queueSnapshotSave();
        }
        function onNextNotificationIdChanged() {
            root.queueSnapshotSave();
        }
    }

    Component.onCompleted: {
        if (!snapshotLoadProc.running)
            snapshotLoadProc.running = true;
    }
}
