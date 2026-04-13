pragma Singleton
import QtQuick

QtObject {
    property string popupMonitorName: ""
    property bool screenshotOverlayOpen: false
    
    property bool showThemeSwitcher: false
    property bool showScreenshotViewer: false
    property string screenshotViewerPath: ""
    property string screenshotViewerMode: ""

    signal lockRequested()
    signal screenshotViewerOpenRequested(string imagePath, string mode)

    function setPopupMonitorName(name) {
        popupMonitorName = String(name || "");
    }

    function parseDaemonPayload(payloadText) {
        const text = String(payloadText || "").trim();
        if (!text.length) return null;
        try {
            return JSON.parse(text);
        } catch (_error) {
            return null;
        }
    }

    // Helper to route daemon snapshots to correct state modules
    function applyDaemonAudioSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        if (payload) AudioState.applyDaemonSnapshot(payload);
    }

    function applyDaemonWifiSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        if (payload) WifiState.applyDaemonSnapshot(payload);
    }

    function applyDaemonBluetoothSnapshot(payloadText) {
        const payload = parseDaemonPayload(payloadText);
        if (payload) BluetoothState.applyDaemonSnapshot(payload);
    }
}
