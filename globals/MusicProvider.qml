pragma Singleton

import QtQuick
import Quickshell.Io
import "."

Item {
    id: musicProvider

    readonly property int pollMs: 1000
    property int consumerCount: 0

    property string musicStatus: "Stopped"
    property string musicPlayer: "N/A"
    property string musicTitle: "Nothing playing"
    property string musicArtist: "N/A"
    property string musicAlbum: "N/A"
    property string musicArtUrl: ""
    property int musicPositionSec: 0
    property int musicLengthSec: 0

    function parseCliJson(raw) {
        try {
            return JSON.parse(String(raw || "").trim());
        } catch (_error) {
            return null;
        }
    }

    function parseTimeToSeconds(timeStr) {
        const str = String(timeStr || "0:00").trim();
        const parts = str.split(":").map(p => parseInt(p, 10) || 0);
        if (parts.length === 2)
            return parts[0] * 60 + parts[1];
        if (parts.length === 3)
            return parts[0] * 3600 + parts[1] * 60 + parts[2];
        return 0;
    }

    function syncGlobalState() {
        GlobalState.musicTitle = musicTitle;
        GlobalState.musicArtist = musicArtist;
        GlobalState.musicAlbum = musicAlbum;
        GlobalState.musicPlayer = musicPlayer;
        GlobalState.musicStatus = musicStatus;
        GlobalState.musicPosition = musicPositionSec;
        GlobalState.musicLength = musicLengthSec;
        GlobalState.musicArtUrl = musicArtUrl;
    }

    function applyMusicPayload(musicPayload) {
        const music = (musicPayload && typeof musicPayload === "object") ? musicPayload : {};

        musicStatus = String(music.status || "Stopped").trim();
        musicPlayer = String(music.player || "N/A").trim();
        musicTitle = String(music.title || "Nothing playing").trim();
        musicArtist = String(music.artist || "N/A").trim();
        musicAlbum = String(music.album || "N/A").trim();
        musicArtUrl = String(music.art_url || "").trim();

        const posSec = Number(music.position_sec);
        const lenSec = Number(music.length_sec);
        musicPositionSec = isNaN(posSec) ? parseTimeToSeconds(music.position || "0:00") : Math.max(0, Math.round(posSec));
        musicLengthSec = isNaN(lenSec) ? parseTimeToSeconds(music.length || "0:00") : Math.max(0, Math.round(lenSec));

        syncGlobalState();
    }

    function refreshNow() {
        if (!musicProc.running)
            musicProc.running = true;
    }

    function acquire() {
        consumerCount = consumerCount + 1;
        if (consumerCount === 1)
            refreshNow();
    }

    function release() {
        consumerCount = Math.max(0, consumerCount - 1);
        if (consumerCount === 0)
            refreshTimer.stop();
    }

    function mediaPlay() {
        mediaProc.command = ["playerctl", "play"];
        mediaProc.running = true;
    }

    function mediaPause() {
        mediaProc.command = ["playerctl", "pause"];
        mediaProc.running = true;
    }

    function mediaPlayPause() {
        mediaProc.command = ["playerctl", "play-pause"];
        mediaProc.running = true;
    }

    function mediaPrevious() {
        mediaProc.command = ["playerctl", "previous"];
        mediaProc.running = true;
    }

    function mediaNext() {
        mediaProc.command = ["playerctl", "next"];
        mediaProc.running = true;
    }

    Process {
        id: musicProc
        command: ["stratum-cli", "dashboard", "music"]
        stdout: StdioCollector {
            onStreamFinished: {
                const payload = musicProvider.parseCliJson(this.text.trim());
                if (payload && payload.ok === true) {
                    const music = (payload.music && typeof payload.music === "object") ? payload.music : payload;
                    musicProvider.applyMusicPayload(music);
                }

                if (musicProvider.consumerCount > 0)
                    refreshTimer.start();
            }
        }
    }

    Timer {
        id: refreshTimer
        interval: musicProvider.pollMs
        repeat: false
        onTriggered: {
            if (musicProvider.consumerCount > 0)
                musicProvider.refreshNow();
        }
    }

    Process {
        id: mediaProc
        stdout: StdioCollector {}
    }
}
