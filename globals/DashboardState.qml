pragma Singleton
import QtQuick

QtObject {
    property bool showMenu: false
    property var lastSnapshot: null

    function applyDaemonSnapshot(payloadText) {
        try {
            const payload = JSON.parse(payloadText);
            lastSnapshot = (payload && payload.dashboard) ? payload.dashboard : payload;
        } catch (e) {
            console.warn("Failed to parse dashboard snapshot:", e);
        }
    }
}
