pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Io

import "../globals"
import "../components"

ApplicationWindow {
    id: audioMenu

    title: "Audio Menu"
    width: 900
    height: 700
    minimumWidth: 820
    minimumHeight: 620
    flags: Qt.Window | Qt.WindowStaysOnTopHint

    visible: GlobalState.showAudioMenu
    onVisibleChanged: {
        if (visible) {
            loadDevices();
            devicesRefreshTimer.start();
            MusicProvider.acquire();
        } else {
            GlobalState.showAudioMenu = false;
            devicesRefreshTimer.stop();
            MusicProvider.release();
        }
    }

    // EQ state
    property var eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    property var eqParametricBands: []
    property real eqPreampDb: 0
    property var eqPresets: []
    property var eqCapabilities: ({})
    property bool eqApplyDryRun: false
    property bool eqApplyOk: true
    property string eqStatusMsg: ""
    property int eqSelectedBandIndex: 0
    property string currentPresetName: "Custom"
    property bool isCustomPreset: true
    property string currentDevice: "@DEFAULT_SINK@"
    property string currentDeviceLabel: "Default Output"

    // UI state
    property bool loading: false
    property string errorMsg: ""
    property var outputDevices: []
    property var inputDevices: []
    property string defaultOutputName: ""
    property string defaultInputName: ""
    property bool routeSwitching: false
    property string routeSwitchKind: ""
    property string routeStatusMsg: ""
    readonly property string musicStatus: MusicProvider.musicStatus
    readonly property string musicTitle: MusicProvider.musicTitle
    readonly property string musicArtist: MusicProvider.musicArtist
    readonly property string musicAlbum: MusicProvider.musicAlbum
    readonly property string musicArtUrl: MusicProvider.musicArtUrl
    readonly property int musicPositionSec: MusicProvider.musicPositionSec
    readonly property int musicLengthSec: MusicProvider.musicLengthSec
    readonly property int devicePollMs: 5000

    readonly property var eqFrequencies: ["31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz", "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"]
    readonly property var eqFilterTypes: ["peaking", "low_shelf", "high_shelf", "low_pass", "high_pass", "band_pass"]
    readonly property int eqMinGain: -12
    readonly property int eqMaxGain: 12
    readonly property real eqGraphMinFreqHz: 20
    readonly property real eqGraphMaxFreqHz: 20000
    readonly property real eqGraphMinDb: -24
    readonly property real eqGraphMaxDb: 24

    function parseCliJson(raw) {
        try {
            return JSON.parse(String(raw || "").trim());
        } catch (_error) {
            return null;
        }
    }

    function closeMenu() {
        if (savePresetDialog.opened) {
            savePresetDialog.close();
            return;
        }
        GlobalState.showAudioMenu = false;
    }

    Shortcut {
        sequence: "Esc"
        context: Qt.WindowShortcut
        enabled: audioMenu.visible
        onActivated: audioMenu.closeMenu()
    }

    function formatTime(seconds) {
        if (seconds < 0 || isNaN(seconds))
            return "00:00";
        const mins = Math.floor(seconds / 60);
        const secs = Math.floor(seconds % 60);
        return (mins < 10 ? "0" : "") + mins + ":" + (secs < 10 ? "0" : "") + secs;
    }

    function formatEqFrequencyLabel(index) {
        const band = Array.isArray(eqParametricBands) ? eqParametricBands[index] : null;
        const hz = Number(band?.frequency_hz);
        if (!isFinite(hz) || hz <= 0)
            return eqFrequencies[index] || "Band";
        if (hz >= 1000)
            return (Math.round((hz / 1000) * 10) / 10) + " kHz";
        return Math.round(hz) + " Hz";
    }

    function log10(value) {
        const v = Number(value);
        if (!isFinite(v) || v <= 0)
            return 0;
        return Math.log(v) / Math.log(10);
    }

    function eqGraphFrequencyAt(normalized) {
        const minF = eqGraphMinFreqHz;
        const maxF = eqGraphMaxFreqHz;
        const n = Math.max(0, Math.min(1, Number(normalized) || 0));
        const minLog = log10(minF);
        const maxLog = log10(maxF);
        const freqLog = minLog + (maxLog - minLog) * n;
        return Math.pow(10, freqLog);
    }

    function eqGraphBandContributionDb(freq, band) {
        if (!band || band.enabled === false)
            return 0;

        const f = Math.max(eqGraphMinFreqHz, Math.min(eqGraphMaxFreqHz, Number(freq) || 1000));
        const center = Math.max(eqGraphMinFreqHz, Math.min(eqGraphMaxFreqHz, Number(band.frequency_hz) || 1000));
        const gain = Number(band.gain_db) || 0;
        const q = Math.max(0.1, Math.min(10, Number(band.q) || 0.707));
        const t = String(band.filter_type || "peaking");

        const distOct = Math.log(f / center) / Math.log(2);
        const widthOct = Math.max(0.08, 1.2 / q);
        const bell = Math.exp(-(distOct * distOct) / (2 * widthOct * widthOct));

        const slope = Math.max(0.6, Math.min(12, q * 2.0));
        const sig = 1 / (1 + Math.exp(-distOct * slope));

        if (t === "low_shelf")
            return gain * (1 - sig);
        if (t === "high_shelf")
            return gain * sig;
        if (t === "low_pass")
            return gain * (1 - sig);
        if (t === "high_pass")
            return gain * sig;

        // peaking and band_pass use a bell-style visual contribution.
        return gain * bell;
    }

    function eqGraphResponseDb(freq) {
        const bands = Array.isArray(eqParametricBands) ? eqParametricBands : [];
        let total = Number(eqPreampDb) || 0;
        for (let i = 0; i < bands.length; i++)
            total += eqGraphBandContributionDb(freq, bands[i]);
        return Math.max(eqGraphMinDb, Math.min(eqGraphMaxDb, total));
    }

    function loadDevices() {
        loading = true;
        errorMsg = "";
        devicesProc.running = true;
    }

    function applyPreset(presetName) {
        eqStatusMsg = "Applying preset...";
        eqApplyOk = true;
        eqApplyDryRun = false;
        applyPresetProc.command = ["stratum-cli", "audio", "equalizer", "apply-preset", audioMenu.currentDevice, presetName];
        applyPresetProc.running = true;
        currentPresetName = presetName;
        isCustomPreset = presetName === "Custom";
    }

    function makeParametricBandsFromLegacy(gains) {
        const defaults = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
        const src = Array.isArray(gains) ? gains : [];
        return defaults.map((freq, idx) => ({
                    frequency_hz: freq,
                    gain_db: Number(src[idx]) || 0,
                    q: 0.707,
                    filter_type: "peaking",
                    enabled: true
                }));
    }

    function applyEqStateFromPayloadBands(parametricBands, legacyBands, preampDb) {
        let bands = [];
        if (Array.isArray(parametricBands) && parametricBands.length > 0) {
            bands = parametricBands.map((band, idx) => ({
                        frequency_hz: Number(band.frequency_hz) || [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000][idx] || 1000,
                        gain_db: Number(band.gain_db) || 0,
                        q: Number(band.q) || 0.707,
                        filter_type: String(band.filter_type || "peaking"),
                        enabled: band.enabled !== false
                    }));
        } else {
            bands = makeParametricBandsFromLegacy(legacyBands);
        }

        audioMenu.eqParametricBands = bands;
        audioMenu.eqBands = bands.slice(0, 10).map(b => Math.round(Number(b.gain_db) || 0));
        audioMenu.eqPreampDb = Number(preampDb) || 0;
        if (bands.length === 0)
            audioMenu.eqSelectedBandIndex = -1;
        else
            audioMenu.eqSelectedBandIndex = Math.max(0, Math.min(audioMenu.eqSelectedBandIndex, bands.length - 1));
    }

    function setEqBandGain(index, gainDb) {
        const rounded = Math.round(Number(gainDb) || 0);
        if (index < 0 || index >= 10)
            return;

        audioMenu.eqBands[index] = rounded;

        const current = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.slice() : [];
        if (!current[index]) {
            const seeded = makeParametricBandsFromLegacy(audioMenu.eqBands);
            audioMenu.eqParametricBands = seeded;
            seeded[index].gain_db = rounded;
            return;
        }

        const existing = current[index];
        current[index] = {
            frequency_hz: Number(existing.frequency_hz) || [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000][index],
            gain_db: rounded,
            q: Number(existing.q) || 0.707,
            filter_type: String(existing.filter_type || "peaking"),
            enabled: existing.enabled !== false
        };
        audioMenu.eqParametricBands = current;
    }

    function syncLegacyEqBandsFromParametric() {
        const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands : [];
        const out = [];
        for (let i = 0; i < 10; i++) {
            const gain = bands[i] ? Number(bands[i].gain_db) : 0;
            out.push(Math.round(isNaN(gain) ? 0 : gain));
        }
        audioMenu.eqBands = out;
    }

    function setParametricBandField(index, field, value) {
        const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.slice() : [];
        if (index < 0 || index >= bands.length)
            return;

        const existing = bands[index] || {};
        const next = {
            frequency_hz: Number(existing.frequency_hz) || 1000,
            gain_db: Number(existing.gain_db) || 0,
            q: Number(existing.q) || 0.707,
            filter_type: String(existing.filter_type || "peaking"),
            enabled: existing.enabled !== false
        };

        if (field === "frequency_hz")
            next.frequency_hz = Math.max(20, Math.min(20000, Number(value) || 1000));
        else if (field === "gain_db")
            next.gain_db = Math.max(audioMenu.eqMinGain, Math.min(audioMenu.eqMaxGain, Number(value) || 0));
        else if (field === "q")
            next.q = Math.max(0.1, Math.min(10, Number(value) || 0.707));
        else if (field === "filter_type")
            next.filter_type = String(value || "peaking");
        else if (field === "enabled")
            next.enabled = !!value;

        bands[index] = next;
        audioMenu.eqParametricBands = bands;
        audioMenu.syncLegacyEqBandsFromParametric();
        audioMenu.currentPresetName = "Custom";
        audioMenu.isCustomPreset = true;
    }

    function addEqBand() {
        const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.slice() : [];
        if (bands.length >= 24)
            return;

        const nextFreq = bands.length > 0 ? Math.min(20000, Math.round((Number(bands[bands.length - 1].frequency_hz) || 1000) * 1.35)) : 1000;
        bands.push({
            frequency_hz: nextFreq,
            gain_db: 0,
            q: 0.707,
            filter_type: "peaking",
            enabled: true
        });
        audioMenu.eqParametricBands = bands;
        audioMenu.eqSelectedBandIndex = bands.length - 1;
        audioMenu.syncLegacyEqBandsFromParametric();
        audioMenu.currentPresetName = "Custom";
        audioMenu.isCustomPreset = true;
    }

    function removeEqBand(index) {
        const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.slice() : [];
        if (bands.length <= 1 || index < 0 || index >= bands.length)
            return;

        bands.splice(index, 1);
        audioMenu.eqParametricBands = bands;
        audioMenu.eqSelectedBandIndex = Math.max(0, Math.min(audioMenu.eqSelectedBandIndex, bands.length - 1));
        audioMenu.syncLegacyEqBandsFromParametric();
        audioMenu.currentPresetName = "Custom";
        audioMenu.isCustomPreset = true;
    }

    function saveCurrentPreset(name) {
        const cleanName = String(name || "").trim();
        if (!cleanName.length)
            return;

        const bands = (Array.isArray(audioMenu.eqParametricBands) && audioMenu.eqParametricBands.length > 0) ? audioMenu.eqParametricBands : makeParametricBandsFromLegacy(audioMenu.eqBands);
        const payload = JSON.stringify({
            bands: bands,
            preamp_db: Number(audioMenu.eqPreampDb) || 0
        });
        savePresetProc.command = ["stratum-cli", "audio", "equalizer", "save-preset-parametric", audioMenu.currentDevice, cleanName, payload];
        savePresetProc.running = true;
        currentPresetName = cleanName;
        isCustomPreset = false;
        eqStatusMsg = "Saving preset...";
    }

    function applyCurrentEq() {
        const bands = (Array.isArray(audioMenu.eqParametricBands) && audioMenu.eqParametricBands.length > 0) ? audioMenu.eqParametricBands : makeParametricBandsFromLegacy(audioMenu.eqBands);
        const payload = JSON.stringify({
            bands: bands,
            preamp_db: Number(audioMenu.eqPreampDb) || 0
        });

        eqStatusMsg = "Applying current EQ...";
        eqApplyOk = true;
        eqApplyDryRun = false;
        currentPresetName = "Custom";
        isCustomPreset = true;

        applyPresetProc.command = ["stratum-cli", "audio", "equalizer", "apply-parametric", audioMenu.currentDevice, payload];
        applyPresetProc.running = true;
    }

    function deletePreset(name) {
        if (!name || name === "Flat" || name === "Bass Boost" || name === "Bright" || name === "Treble Boost")
            return;

        deletePresetProc.command = ["stratum-cli", "audio", "equalizer", "delete-preset", audioMenu.currentDevice, name];
        deletePresetProc.running = true;
    }

    function resetToFlat() {
        for (let i = 0; i < 10; i++)
            eqBands[i] = 0;

        eqParametricBands = makeParametricBandsFromLegacy(eqBands);
        eqPreampDb = 0;

        currentPresetName = "Flat";
        isCustomPreset = false;
        eqStatusMsg = "Reset to Flat.";
        applyPreset("Flat");
    }

    function loadPresetsForDevice() {
        listPresetsProc.command = ["stratum-cli", "audio", "equalizer", "list-presets", audioMenu.currentDevice];
        listPresetsProc.running = true;
    }

    function switchOutput(deviceName, deviceLabel) {
        currentDevice = String(deviceName || "");
        currentDeviceLabel = String(deviceLabel || deviceName || "Default Output");
        loadPresetsForDevice();
    }

    function switchInput(deviceName) {
        const target = String(deviceName || "").trim();
        if (!target.length || routeSwitching || target === defaultInputName)
            return;

        routeSwitching = true;
        routeSwitchKind = "input";
        routeStatusMsg = "Switching input...";
        routeProc.command = ["stratum-cli", "audio", "set-input", target];
        routeProc.running = true;
    }

    function switchOutputRoute(deviceName) {
        const target = String(deviceName || "").trim();
        if (!target.length || routeSwitching || target === defaultOutputName)
            return;

        routeSwitching = true;
        routeSwitchKind = "output";
        routeStatusMsg = "Switching output...";
        routeProc.command = ["stratum-cli", "audio", "set-output", target];
        routeProc.running = true;
    }

    function hasPlayableMusic() {
        const status = String(musicStatus || "");
        if (status === "Playing" || status === "Paused")
            return true;
        return musicTitle.length > 0 && musicTitle !== "Nothing playing";
    }

    function deviceLabel(name, description) {
        if (description && String(description).trim().length > 0)
            return String(description).trim();
        return String(name || "");
    }

    Process {
        id: devicesProc
        command: ["stratum-cli", "audio", "status", "--hover"]
        stdout: StdioCollector {
            onStreamFinished: {
                audioMenu.loading = false;
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (!payload || payload.ok !== true) {
                    audioMenu.errorMsg = payload?.error || "Failed to load devices";
                    return;
                }

                const sinks = Array.isArray(payload.sinks) ? payload.sinks : [];
                const sources = Array.isArray(payload.sources) ? payload.sources : [];
                const defaults = payload.default || {};

                audioMenu.outputDevices = sinks.filter(s => String(s.name || "").trim()).map(s => ({
                            name: String(s.name || "").trim(),
                            description: String(s.description || "").trim()
                        }));

                audioMenu.inputDevices = sources.filter(s => String(s.name || "").trim()).map(s => ({
                            name: String(s.name || "").trim(),
                            description: String(s.description || "").trim()
                        }));

                audioMenu.defaultOutputName = String(defaults.sink || "");
                audioMenu.defaultInputName = String(defaults.source || "");

                const defaultSink = audioMenu.outputDevices.find(d => d.name === audioMenu.defaultOutputName);
                if (defaultSink) {
                    audioMenu.currentDevice = defaultSink.name;
                    audioMenu.currentDeviceLabel = defaultSink.description || defaultSink.name;
                }
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.palette.bgMain
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 24
        spacing: 18

        RowLayout {
            Layout.fillWidth: true
            spacing: 14

            Text {
                text: "Audio Settings"
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 23
                font.bold: true
                font.letterSpacing: 0.3
            }

            Item {
                Layout.fillWidth: true
            }

            Rectangle {
                id: closeButton
                Layout.preferredHeight: 34
                Layout.preferredWidth: 40
                radius: 6
                color: closeMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                Text {
                    anchors.centerIn: parent
                    text: "󰅖"
                    color: Theme.palette.error
                    font.family: Theme.palette.font
                    font.pixelSize: 13
                }

                MouseArea {
                    id: closeMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: audioMenu.closeMenu()
                }
            }
        }

        TabBar {
            id: tabBar
            Layout.fillWidth: true
            spacing: 8

            background: Rectangle {
                color: Theme.palette.bgDark
                radius: 12
            }

            TabButton {
                text: "Playback"
                implicitHeight: 40
                hoverEnabled: true
                contentItem: Text {
                    text: parent.text
                    color: parent.checked ? Theme.palette.textMain : Theme.palette.textMuted
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                    font.bold: parent.checked
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                    radius: 10
                    border.color: parent.checked ? Theme.palette.borderActive : Theme.palette.borderInactive
                    border.width: 1
                }
            }

            TabButton {
                text: "Equalizer"
                implicitHeight: 40
                hoverEnabled: true
                contentItem: Text {
                    text: parent.text
                    color: parent.checked ? Theme.palette.textMain : Theme.palette.textMuted
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                    font.bold: parent.checked
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                    radius: 10
                    border.color: parent.checked ? Theme.palette.borderActive : Theme.palette.borderInactive
                    border.width: 1
                }
            }
        }

        StackLayout {
            currentIndex: tabBar.currentIndex
            Layout.fillWidth: true
            Layout.fillHeight: true

            Rectangle {
                color: Theme.palette.bgMain
                radius: 14
                border.color: Theme.palette.borderInactive
                border.width: 1

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 16
                    spacing: 12

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        spacing: 20

                        Item {
                            Layout.fillHeight: true
                        }

                        RowLayout {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 700
                            spacing: 18

                            Rectangle {
                                Layout.alignment: Qt.AlignBottom
                                implicitWidth: 400
                                implicitHeight: 400
                                radius: 14
                                color: Theme.palette.bgWidget
                                border.color: Theme.palette.borderInactive
                                border.width: 1

                                Image {
                                    id: albumArt
                                    anchors.fill: parent
                                    source: audioMenu.musicArtUrl
                                    fillMode: Image.PreserveAspectCrop
                                    asynchronous: true
                                }

                                Text {
                                    anchors.centerIn: parent
                                    visible: audioMenu.musicArtUrl == ""
                                    text: "󰎆"
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 75
                                }
                            }

                            ColumnLayout {
                                Layout.alignment: Qt.AlignBottom
                                Layout.fillWidth: true
                                spacing: 12

                                Item {
                                    id: marqueeRoot
                                    Layout.fillWidth: true
                                    implicitHeight: 28
                                    clip: true

                                    readonly property int marqueeGap: 42
                                    readonly property bool needsMarquee: titleText.implicitWidth > width

                                    Row {
                                        id: titleMarqueeRow
                                        x: 0
                                        y: (parent.height - height) / 2
                                        spacing: parent.marqueeGap

                                        Text {
                                            id: titleText
                                            text: audioMenu.hasPlayableMusic() ? (audioMenu.musicTitle || "Unknown Title") : "Nothing is playing right now"
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 18
                                            font.bold: true
                                            wrapMode: Text.NoWrap
                                        }

                                        Text {
                                            visible: marqueeRoot.needsMarquee
                                            text: titleText.text
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 18
                                            font.bold: true
                                            wrapMode: Text.NoWrap
                                        }
                                    }

                                    SequentialAnimation {
                                        running: marqueeRoot.needsMarquee
                                        loops: Animation.Infinite

                                        PauseAnimation {
                                            duration: 650
                                        }

                                        NumberAnimation {
                                            target: titleMarqueeRow
                                            property: "x"
                                            from: 0
                                            to: -(titleText.implicitWidth + marqueeRoot.marqueeGap)
                                            duration: Math.max(3200, Math.round((titleText.implicitWidth + marqueeRoot.marqueeGap) * 24))
                                            easing.type: Easing.Linear
                                        }
                                    }

                                    onNeedsMarqueeChanged: {
                                        if (!needsMarquee)
                                            titleMarqueeRow.x = 0;
                                    }
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: audioMenu.musicArtist || "Unknown Artist"
                                    color: Theme.palette.textMuted
                                    font.family: Theme.palette.font
                                    font.pixelSize: 14
                                    maximumLineCount: 1
                                    horizontalAlignment: Text.AlignLeft
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: audioMenu.musicAlbum || ""
                                    color: Theme.palette.textMuted
                                    font.family: Theme.palette.font
                                    font.pixelSize: 12
                                    wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                                    maximumLineCount: 1
                                    horizontalAlignment: Text.AlignLeft
                                    visible: audioMenu.musicAlbum && audioMenu.musicAlbum !== "N/A"
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 8

                                    Slider {
                                        id: menuSeekSlider
                                        from: 0
                                        to: Math.max(1, audioMenu.musicLengthSec)
                                        value: audioMenu.musicPositionSec
                                        Layout.fillWidth: true

                                        background: Rectangle {
                                            x: menuSeekSlider.leftPadding
                                            y: menuSeekSlider.topPadding + menuSeekSlider.availableHeight / 2 - height / 2
                                            width: menuSeekSlider.availableWidth
                                            height: 4
                                            radius: 2
                                            color: Theme.palette.bgHover

                                            Rectangle {
                                                width: menuSeekSlider.visualPosition * parent.width
                                                height: parent.height
                                                radius: 2
                                                color: Theme.palette.primary
                                            }
                                        }

                                        handle: Rectangle {
                                            x: menuSeekSlider.leftPadding + menuSeekSlider.visualPosition * (menuSeekSlider.availableWidth - width)
                                            y: menuSeekSlider.topPadding + menuSeekSlider.availableHeight / 2 - height / 2
                                            implicitWidth: 12
                                            implicitHeight: 12
                                            radius: 6
                                            color: Theme.palette.primary

                                            MouseArea {
                                                anchors.fill: parent
                                                anchors.margins: -4
                                                onReleased: {
                                                    const newPos = Math.round(menuSeekSlider.value);
                                                    menuSeekProc.command = ["stratum-cli", "audio", "media", "seek", String(newPos)];
                                                    menuSeekProc.running = true;
                                                }
                                            }
                                        }
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true

                                        Text {
                                            text: formatTime(audioMenu.musicPositionSec)
                                            color: Theme.palette.textMuted
                                            font.family: Theme.palette.font
                                            font.pixelSize: 11
                                        }

                                        Item {
                                            Layout.fillWidth: true
                                        }

                                        Text {
                                            text: formatTime(audioMenu.musicLengthSec)
                                            color: Theme.palette.textMuted
                                            font.family: Theme.palette.font
                                            font.pixelSize: 11
                                        }
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: 12

                                    Button {
                                        text: ""
                                        Layout.fillWidth: true
                                        implicitHeight: 40
                                        hoverEnabled: true
                                        background: Rectangle {
                                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                                            border.width: 1
                                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                                            radius: 8
                                        }
                                        contentItem: Text {
                                            text: parent.text
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 30
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                        onClicked: MusicProvider.mediaPrevious()
                                    }

                                    Button {
                                        text: audioMenu.musicStatus === "Playing" ? "" : ""

                                        Layout.fillWidth: true
                                        implicitHeight: 40
                                        hoverEnabled: true
                                        background: Rectangle {
                                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                                            border.width: 1
                                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                                            radius: 8
                                        }
                                        contentItem: Text {
                                            text: parent.text
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 30
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                        onClicked: MusicProvider.mediaPlayPause()
                                    }

                                    Button {
                                        text: ""
                                        Layout.fillWidth: true
                                        implicitHeight: 40
                                        hoverEnabled: true
                                        background: Rectangle {
                                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                                            border.width: 1
                                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                                            radius: 8
                                        }
                                        contentItem: Text {
                                            text: parent.text
                                            color: Theme.palette.textMain
                                            font.family: Theme.palette.font
                                            font.pixelSize: 30
                                            horizontalAlignment: Text.AlignHCenter
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                        onClicked: MusicProvider.mediaNext()
                                    }
                                }
                            }
                        }

                        RowLayout {
                            id: routeSelectorsRow
                            Layout.fillWidth: true
                            spacing: 10

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.preferredWidth: 1
                                Layout.minimumWidth: 0
                                spacing: 6

                                Text {
                                    Layout.fillWidth: true
                                    text: "Audio Input"
                                    color: Theme.palette.textMuted
                                    font.family: Theme.palette.font
                                    font.pixelSize: 11
                                }

                                ThemedComboBox {
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    items: audioMenu.inputDevices
                                    selectedName: audioMenu.defaultInputName
                                    placeholderText: "Select input device"
                                    labelProvider: item => audioMenu.deviceLabel(item?.name, item?.description)
                                    onItemChosen: item => audioMenu.switchInput(item?.name)
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.preferredWidth: 1
                                Layout.minimumWidth: 0
                                spacing: 6

                                Text {
                                    Layout.fillWidth: true
                                    text: "Audio Output"
                                    color: Theme.palette.textMuted
                                    font.family: Theme.palette.font
                                    font.pixelSize: 11
                                }

                                ThemedComboBox {
                                    Layout.fillWidth: true
                                    Layout.minimumWidth: 0
                                    items: audioMenu.outputDevices
                                    selectedName: audioMenu.defaultOutputName
                                    placeholderText: "Select output device"
                                    labelProvider: item => audioMenu.deviceLabel(item?.name, item?.description)
                                    onItemChosen: item => audioMenu.switchOutputRoute(item?.name)
                                }
                            }
                        }
                    }
                }
            }

            Rectangle {
                color: Theme.palette.bgMain
                radius: 14
                border.color: Theme.palette.borderInactive
                border.width: 1

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 10

                    AudioEqualizerPanel {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        audioMenu: audioMenu
                    }
                }
                Item {
                    Layout.fillHeight: true
                }
            }
        }
    }

    Popup {
        id: savePresetDialog
        anchors.centerIn: parent
        width: 320
        height: 156
        modal: true

        background: Rectangle {
            color: audioMenu.surfacePrimary
            radius: 12
            border.color: audioMenu.strokeStrong
            border.width: 1
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 12

            Text {
                text: "Save preset as"
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 14
                font.bold: true
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 36
                color: Qt.rgba(0, 0, 0, 0.2)
                border.color: audioMenu.strokeStrong
                border.width: 1
                radius: 8

                TextInput {
                    id: presetNameInput
                    anchors.fill: parent
                    anchors.margins: 8
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                    clip: true
                    selectedTextColor: Theme.palette.bgMain
                    selectionColor: Theme.palette.primary

                    Keys.onReturnPressed: {
                        audioMenu.saveCurrentPreset(text);
                        savePresetDialog.close();
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Button {
                    text: "Save"
                    Layout.fillWidth: true
                    hoverEnabled: true
                    background: Rectangle {
                        color: parent.hovered ? Qt.lighter(Theme.palette.primary, 1.1) : Theme.palette.primary
                        radius: 8
                    }
                    contentItem: Text {
                        text: parent.text
                        color: Theme.palette.bgMain
                        font.family: Theme.palette.font
                        horizontalAlignment: Text.AlignHCenter
                    }
                    onClicked: {
                        audioMenu.saveCurrentPreset(presetNameInput.text);
                        savePresetDialog.close();
                    }
                }

                Button {
                    text: "Cancel"
                    Layout.fillWidth: true
                    hoverEnabled: true
                    background: Rectangle {
                        color: parent.hovered ? Qt.lighter(Theme.palette.secondary, 1.08) : Theme.palette.secondary
                        radius: 8
                    }
                    contentItem: Text {
                        text: parent.text
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        horizontalAlignment: Text.AlignHCenter
                    }
                    onClicked: savePresetDialog.close()
                }
            }
        }

        onOpened: {
            presetNameInput.text = "";
            presetNameInput.forceActiveFocus();
        }
    }

    Process {
        id: menuSeekProc
        stdout: StdioCollector {}
    }

    Process {
        id: routeProc
        stdout: StdioCollector {
            onStreamFinished: {
                audioMenu.routeSwitching = false;
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (!payload || payload.ok !== true) {
                    audioMenu.routeStatusMsg = payload?.error || "Failed to switch input";
                    return;
                }

                audioMenu.routeStatusMsg = audioMenu.routeSwitchKind === "output" ? "Output switched" : "Input switched";
                audioMenu.routeSwitchKind = "";
                audioMenu.loadDevices();
            }
        }
    }

    Timer {
        id: devicesRefreshTimer
        interval: audioMenu.devicePollMs
        repeat: true
        onTriggered: {
            if (audioMenu.visible && !audioMenu.loading && !devicesProc.running)
                audioMenu.loadDevices();
        }
    }

    Process {
        id: listPresetsProc
        stdout: StdioCollector {
            onStreamFinished: {
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (!payload || payload.ok !== true)
                    return;

                const presets = Array.isArray(payload.presets) ? payload.presets : [];
                audioMenu.eqPresets = presets;
                audioMenu.eqCapabilities = (payload.capabilities && typeof payload.capabilities === "object") ? payload.capabilities : {};

                const activePresetName = String(payload.active_preset || "").trim();
                if (activePresetName.length)
                    audioMenu.currentPresetName = activePresetName;

                const names = presets.map(p => String(p.name || ""));
                if (!names.includes(audioMenu.currentPresetName)) {
                    audioMenu.currentPresetName = "Flat";
                    audioMenu.isCustomPreset = false;
                }

                const active = presets.find(p => String(p.name || "") === audioMenu.currentPresetName);
                if (active) {
                    const parametricBands = Array.isArray(active.parametric_bands) ? active.parametric_bands : [];
                    const legacyBands = Array.isArray(active.bands) ? active.bands : [];
                    audioMenu.applyEqStateFromPayloadBands(parametricBands, legacyBands, active.preamp_db);
                }

                audioMenu.eqStatusMsg = "Loaded " + presets.length + " preset" + (presets.length === 1 ? "" : "s") + " for this output.";
            }
        }
    }

    Process {
        id: applyPresetProc
        stdout: StdioCollector {
            onStreamFinished: {
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (!payload || payload.ok !== true) {
                    audioMenu.eqApplyOk = false;
                    audioMenu.eqApplyDryRun = true;
                    audioMenu.eqStatusMsg = payload?.error || "Failed to apply preset.";
                    return;
                }

                const parametricBands = Array.isArray(payload.parametric_bands) ? payload.parametric_bands : [];
                const legacyBands = Array.isArray(payload.bands) ? payload.bands : [];
                audioMenu.applyEqStateFromPayloadBands(parametricBands, legacyBands, payload.preamp_db);

                const applyInfo = (payload.apply && typeof payload.apply === "object") ? payload.apply : {};
                audioMenu.eqApplyOk = payload.apply_ok !== false;
                audioMenu.eqApplyDryRun = applyInfo.dry_run === true;
                audioMenu.eqStatusMsg = String(payload.status || applyInfo.status || "Preset applied.");
            }
        }
    }

    Process {
        id: savePresetProc
        stdout: StdioCollector {
            onStreamFinished: {
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (payload && payload.ok === true) {
                    audioMenu.eqStatusMsg = "Preset saved.";
                    audioMenu.loadPresetsForDevice();
                } else if (payload) {
                    audioMenu.eqApplyOk = false;
                    audioMenu.eqStatusMsg = payload.error || "Failed to save preset.";
                }
            }
        }
    }

    Process {
        id: deletePresetProc
        stdout: StdioCollector {
            onStreamFinished: {
                const payload = audioMenu.parseCliJson(this.text.trim());
                if (payload && payload.ok === true) {
                    audioMenu.eqStatusMsg = "Preset deleted.";
                    audioMenu.resetToFlat();
                    audioMenu.loadPresetsForDevice();
                } else if (payload) {
                    audioMenu.eqApplyOk = false;
                    audioMenu.eqStatusMsg = payload.error || "Failed to delete preset.";
                }
            }
        }
    }

    Component.onCompleted: {
        audioMenu.eqParametricBands = audioMenu.makeParametricBandsFromLegacy(audioMenu.eqBands);
        audioMenu.loadDevices();
        audioMenu.loadPresetsForDevice();
        if (audioMenu.visible)
            MusicProvider.acquire();
    }

    Component.onDestruction: {
        if (audioMenu.visible)
            MusicProvider.release();
    }
}
