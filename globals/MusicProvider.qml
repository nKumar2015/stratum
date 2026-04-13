pragma Singleton

import QtQuick
import Quickshell.Io
import "."
import "../globals/DaemonRpc.js" as DaemonRpc

Item {
    id: musicProvider

    property int consumerCount: 0

    property string musicStatus: "Stopped"
    property string musicPlayer: "N/A"
    property string musicTitle: "Nothing playing"
    property string musicArtist: "N/A"
    property string musicAlbum: "N/A"
    property string musicArtUrl: ""
    property int musicPositionSec: 0
    property int musicLengthSec: 0

    property bool _isInitialStatusFetched: false

    Timer {
        id: fallbackPollingTimer
        interval: 2500
        repeat: true
        running: consumerCount > 0
        onTriggered: musicProvider.refreshStatusFromDaemon()
    }

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
        AudioState.musicTitle = musicTitle;
        AudioState.musicArtist = musicArtist;
        AudioState.musicAlbum = musicAlbum;
        AudioState.musicPlayer = musicPlayer;
        AudioState.musicStatus = musicStatus;
        AudioState.musicPosition = musicPositionSec;
        AudioState.musicLength = musicLengthSec;
        AudioState.musicArtUrl = musicArtUrl;
    }

    function applyMusicPayload(musicPayload) {
        const music = (musicPayload && typeof musicPayload === "object") ? musicPayload : {};

        musicStatus = String(music.status || "Stopped").trim();
        musicPlayer = String(music.player || "N/A").trim();
        musicTitle = String(music.title || "Nothing playing").trim();
        musicArtist = String(music.artist || "N/A").trim();
        musicAlbum = String(music.album || "N/A").trim();
        musicArtUrl = String(music.art_url || music.artUrl || "").trim();

        const posSec = Number(music.position_sec ?? music.positionSec ?? music.position);
        const lenSec = Number(music.length_sec ?? music.lengthSec ?? music.length);
        
        musicPositionSec = isNaN(posSec) ? parseTimeToSeconds(music.position || "0:00") : Math.max(0, Math.round(posSec));
        musicLengthSec = isNaN(lenSec) ? parseTimeToSeconds(music.length || "0:00") : Math.max(0, Math.round(lenSec));

        syncGlobalState();
    }

    function applyDaemonMusicSnapshot(payloadText) {
        const payload = parseCliJson(payloadText);
        // Daemon might send { music: { ... } } or just the fields depending on the IPC call
        const music = (payload && payload.music && typeof payload.music === "object") ? payload.music : payload;
        if (!music || typeof music !== "object")
            return;

        applyMusicPayload(music);
    }

    function refreshStatusFromDaemon() {
        if (!DaemonRpc.canUse()) return;
        
        refreshProc.command = DaemonRpc.command("audio.media_info");
        refreshProc.running = true;
    }

    Process {
        id: refreshProc
        stdout: StdioCollector {
            onStreamFinished: applyDaemonMusicSnapshot(this.text)
        }
    }

    function acquire() {
        consumerCount = consumerCount + 1;
    }

    function release() {
        consumerCount = Math.max(0, consumerCount - 1);
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
        id: mediaProc
        stdout: StdioCollector {}
    }
}
