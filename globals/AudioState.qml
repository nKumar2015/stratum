pragma Singleton
import QtQuick
import Quickshell.Io

Item {
    id: audioState

    property bool showHoverMenu: false
    property bool showMenu: false
    property real iconY: 0
    property bool hoverIntent: false
    property int volumePercent: 0
    property bool muted: true
    property bool headphonesOutput: false
    property bool userAdjusting: false
    property bool daemonAvailable: true

    property var outputDevices: []
    property var inputDevices: []
    property string defaultOutput: ""
    property string defaultInput: ""

    // Optimistic switch guard: while non-empty, daemon pushes won't overwrite
    // the corresponding default unless they match the pending value (confirming it).
    property string _pendingOutput: ""
    property string _pendingInput: ""

    Timer {
        id: pendingOutputTimer
        interval: 5000
        repeat: false
        onTriggered: audioState._pendingOutput = ""
    }

    Timer {
        id: pendingInputTimer
        interval: 5000
        repeat: false
        onTriggered: audioState._pendingInput = ""
    }

    Timer {
        id: pendingHeadphonesTimer
        interval: 3000
        repeat: false
    }

    function isDeviceHeadphone(name, isOutput) {
        if (!name) return false;
        
        let desc = "";
        const list = isOutput ? outputDevices : inputDevices;
        for (let i = 0; i < list.length; i++) {
            if (list[i] && list[i].name === name) {
                desc = list[i].description || "";
                break;
            }
        }
        
        const probe = String(name + " " + desc).toLowerCase();
        return probe.includes("bluez") || 
               probe.includes("headphone") || 
               probe.includes("headset") || 
               probe.includes("buds") || 
               probe.includes("airpods") || 
               probe.includes("earbud");
    }

    function setOptimisticOutput(name) {
        defaultOutput = name;
        _pendingOutput = name;
        headphonesOutput = isDeviceHeadphone(name, true);
        pendingOutputTimer.restart();
        pendingHeadphonesTimer.restart();
    }

    function setOptimisticInput(name) {
        defaultInput = name;
        _pendingInput = name;
        if (isDeviceHeadphone(name, false)) {
            headphonesOutput = true;
            pendingHeadphonesTimer.restart();
        }
        pendingInputTimer.restart();
    }

    // Music properties (moved from GlobalState)
    property string musicTitle: ""
    property string musicArtist: ""
    property string musicAlbum: ""
    property string musicPlayer: ""
    property string musicStatus: ""
    property int musicPosition: 0
    property int musicLength: 0
    property string musicArtUrl: ""

    /// Compare two device arrays by name+description to avoid unnecessary
    /// QML property reassignment (which tears down and recreates delegates).
    function arraysMatchByName(a, b) {
        if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length)
            return false;
        for (let i = 0; i < a.length; i++) {
            if (String(a[i]?.name || "") !== String(b[i]?.name || ""))
                return false;
            if (String(a[i]?.description || "") !== String(b[i]?.description || ""))
                return false;
        }
        return true;
    }

    function applyDaemonSnapshot(payload) {
        const audio = (payload && payload.audio && typeof payload.audio === "object") ? payload.audio : payload;
        if (!audio || typeof audio !== "object")
            return;

        const status = (audio.status && typeof audio.status === "object") ? audio.status : null;

        if (!userAdjusting) {
            const volumeValue = status && status.volume !== undefined ? status.volume : audio.volume;
            const parsedVolume = parseInt(String(volumeValue || "0").replace("%", ""));
            volumePercent = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(150, parsedVolume));

            const muteValue = status && status.mute !== undefined ? status.mute : audio.mute;
            if (typeof muteValue === "boolean")
                muted = muteValue;
            else
                muted = String(muteValue || "yes").trim().toLowerCase() === "yes";
        }
        const incomingHeadphones = String(audio.headphones || "no").trim().toLowerCase() === "yes";
        if (!pendingHeadphonesTimer.running) {
            headphonesOutput = incomingHeadphones;
        }

        // Device lists — only replace if content actually changed to avoid
        // QML Repeater/ComboBox teardown flicker during EQ graph rebuilds.
        if (Array.isArray(audio.sinks)) {
            const newSinks = audio.sinks.map(function (row) {
                return {
                    name: String(row.name || "").trim(),
                    description: String(row.description || String(row.name || "")).trim()
                };
            }).slice(0, 6);
            if (!arraysMatchByName(outputDevices, newSinks))
                outputDevices = newSinks;
        }

        if (Array.isArray(audio.sources)) {
            const newSources = audio.sources.map(function (row) {
                return {
                    name: String(row.name || "").trim(),
                    description: String(row.description || String(row.name || "")).trim()
                };
            }).slice(0, 6);
            if (!arraysMatchByName(inputDevices, newSources))
                inputDevices = newSources;
        }

        // Support both top-level and nested default sink/source.
        // During EQ teardown/rebuild, transient snapshots may arrive with empty
        // defaults. Preserve the last known-good value to avoid UI flicker.
        const newSink = (typeof audio.default_sink === "string" && audio.default_sink.length > 0)
            ? String(audio.default_sink)
            : (audio.default && typeof audio.default.sink === "string" && audio.default.sink.length > 0)
                ? String(audio.default.sink)
                : "";
        if (newSink.length > 0) {
            if (_pendingOutput.length > 0) {
                // Daemon confirmed the optimistic value — clear the guard.
                if (newSink === _pendingOutput) {
                    _pendingOutput = "";
                    pendingOutputTimer.stop();
                }
                // Otherwise ignore stale daemon value while guard is active.
            } else {
                defaultOutput = newSink;
            }
        }

        const newSource = (typeof audio.default_source === "string" && audio.default_source.length > 0)
            ? String(audio.default_source)
            : (audio.default && typeof audio.default.source === "string" && audio.default.source.length > 0)
                ? String(audio.default.source)
                : "";
        if (newSource.length > 0) {
            if (_pendingInput.length > 0) {
                if (newSource === _pendingInput) {
                    _pendingInput = "";
                    pendingInputTimer.stop();
                }
            } else {
                defaultInput = newSource;
            }
        }
    }
}
