pragma Singleton
import QtQuick

QtObject {
    property bool showMenu: false
    property bool showHoverMenu: false
    property real iconY: 0
    property bool hoverIntent: false
    property string state: "disconnected"
    property int signalPercent: 0
    property bool ethernet: false
    property bool enabled: false

    function applyDaemonSnapshot(payload) {
        const wifi = (payload && payload.net && typeof payload.net === "object") ? payload.net : payload;
        if (!wifi || typeof wifi !== "object")
            return;

        const s = String(wifi.state || "disconnected").trim().toLowerCase();
        state = s;
        ethernet = s === "ethernet";
        enabled = s === "ethernet" || s === "wifi";

        const signal = parseInt(String(wifi.signal_pct || "0"));
        signalPercent = isNaN(signal) ? 0 : Math.max(0, Math.min(100, signal));
    }
}
