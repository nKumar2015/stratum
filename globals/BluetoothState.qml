pragma Singleton
import QtQuick

QtObject {
    property bool showMenu: false
    property bool showHoverMenu: false
    property real iconY: 0
    property bool hoverIntent: false
    property bool powered: false
    property bool connected: false
    property bool scanning: false

    function applyDaemonSnapshot(payload) {
        const bluetooth = (payload && payload.bluetooth && typeof payload.bluetooth === "object") ? payload.bluetooth : payload;
        if (!bluetooth || typeof bluetooth !== "object")
            return;

        const raw = String(bluetooth.state || "off").trim().toLowerCase();
        const pwr = String(bluetooth.powered || "no").trim().toLowerCase();
        
        powered = (pwr === "yes") || (raw === "connected" || raw === "on");
        connected = raw === "connected";
        scanning = String(bluetooth.scanning || "no").trim().toLowerCase() === "yes";
    }
}
