pragma Singleton
import QtQuick

QtObject {
    property bool showHoverMenu: false
    property bool showMenu: false
    property real iconY: 0
    property bool hoverIntent: false
    property int volumePercent: 0
    property bool muted: true
    property bool headphonesOutput: false
    property bool userAdjusting: false
    property bool daemonAvailable: true

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

        const parsedVolume = parseInt(String(audio.volume || "0").replace("%", ""));
        volumePercent = isNaN(parsedVolume) ? 0 : Math.max(0, Math.min(150, parsedVolume));
        muted = String(audio.mute || "yes").trim().toLowerCase() === "yes";
        headphonesOutput = String(audio.headphones || "no").trim().toLowerCase() === "yes";
    }
}
