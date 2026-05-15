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

    // Music properties (moved from GlobalState)
    property string musicTitle: ""
    property string musicArtist: ""
    property string musicAlbum: ""
    property string musicPlayer: ""
    property string musicStatus: ""
    property int musicPosition: 0
    property int musicLength: 0
    property string musicArtUrl: ""

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
        headphonesOutput = String(audio.headphones || "no").trim().toLowerCase() === "yes";

        // Device lists
        if (Array.isArray(audio.sinks)) {
            outputDevices = audio.sinks.map(function (row) {
                return {
                    name: String(row.name || "").trim(),
                    description: String(row.description || String(row.name || "")).trim()
                };
            }).slice(0, 6);
        }

        if (Array.isArray(audio.sources)) {
            inputDevices = audio.sources.map(function (row) {
                return {
                    name: String(row.name || "").trim(),
                    description: String(row.description || String(row.name || "")).trim()
                };
            }).slice(0, 6);
        }

        // Support both top-level and nested default sink/source
        if (typeof audio.default_sink === "string" && audio.default_sink.length > 0) {
            defaultOutput = String(audio.default_sink);
        } else if (audio.default && typeof audio.default.sink === "string") {
            defaultOutput = String(audio.default.sink);
        } else {
            defaultOutput = "";
        }

        if (typeof audio.default_source === "string" && audio.default_source.length > 0) {
            defaultInput = String(audio.default_source);
        } else if (audio.default && typeof audio.default.source === "string") {
            defaultInput = String(audio.default.source);
        } else {
            defaultInput = "";
        }
    }
}
