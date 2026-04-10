import QtQuick
import QtQuick.Layouts
import QtQuick.Controls

import "../globals"
import "../components"

Rectangle {
    id: eqPanel
    required property var audioMenu
    color: "transparent"

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        Text {
            text: "Parametric EQ"
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 13
            font.bold: true
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 14
            ColumnLayout {
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredWidth: 680
                    radius: 8
                    color: Theme.palette.bgWidget
                    border.color: Theme.palette.borderInactive
                    border.width: 1

                    Canvas {
                        id: eqGraphCanvas
                        anchors.fill: parent
                        anchors.margins: 8
                        antialiasing: true

                        onPaint: {
                            const ctx = getContext("2d");
                            const w = width;
                            const h = height;
                            ctx.reset();
                            ctx.clearRect(0, 0, w, h);

                            const leftPad = 42;
                            const rightPad = 12;
                            const topPad = 14;
                            const bottomPad = 26;
                            const chartW = Math.max(10, w - leftPad - rightPad);
                            const chartH = Math.max(10, h - topPad - bottomPad);

                            function xAtFreq(freq) {
                                const minL = audioMenu.log10(audioMenu.eqGraphMinFreqHz);
                                const maxL = audioMenu.log10(audioMenu.eqGraphMaxFreqHz);
                                const curL = audioMenu.log10(Math.max(audioMenu.eqGraphMinFreqHz, Math.min(audioMenu.eqGraphMaxFreqHz, freq)));
                                const t = (curL - minL) / Math.max(0.0001, maxL - minL);
                                return leftPad + t * chartW;
                            }

                            function yAtDb(db) {
                                const clamped = Math.max(audioMenu.eqGraphMinDb, Math.min(audioMenu.eqGraphMaxDb, db));
                                const t = (clamped - audioMenu.eqGraphMinDb) / Math.max(0.0001, audioMenu.eqGraphMaxDb - audioMenu.eqGraphMinDb);
                                return topPad + (1 - t) * chartH;
                            }

                            const dbTicks = [-24, -12, 0, 12, 24];
                            for (let i = 0; i < dbTicks.length; i++) {
                                const db = dbTicks[i];
                                const y = yAtDb(db);
                                ctx.beginPath();
                                ctx.moveTo(leftPad, y);
                                ctx.lineTo(leftPad + chartW, y);
                                ctx.lineWidth = db === 0 ? 1.5 : 1;
                                ctx.strokeStyle = db === 0 ? Theme.palette.primary : Qt.rgba(1, 1, 1, 0.14);
                                ctx.stroke();

                                ctx.fillStyle = Theme.palette.textMuted;
                                ctx.font = "10px " + Theme.palette.font;
                                ctx.textAlign = "right";
                                ctx.textBaseline = "middle";
                                ctx.fillText((db > 0 ? "+" : "") + db, leftPad - 6, y);
                            }

                            const freqTicks = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
                            for (let i = 0; i < freqTicks.length; i++) {
                                const f = freqTicks[i];
                                const x = xAtFreq(f);
                                ctx.beginPath();
                                ctx.moveTo(x, topPad);
                                ctx.lineTo(x, topPad + chartH);
                                ctx.lineWidth = 1;
                                ctx.strokeStyle = Qt.rgba(1, 1, 1, 0.10);
                                ctx.stroke();

                                ctx.fillStyle = Theme.palette.textMuted;
                                ctx.font = "9px " + Theme.palette.font;
                                ctx.textAlign = "center";
                                ctx.textBaseline = "top";
                                ctx.fillText(f >= 1000 ? (f / 1000) + "k" : String(f), x, topPad + chartH + 4);
                            }

                            const points = 320;
                            ctx.beginPath();
                            for (let i = 0; i < points; i++) {
                                const t = i / (points - 1);
                                const freq = audioMenu.eqGraphFrequencyAt(t);
                                const db = audioMenu.eqGraphResponseDb(freq);
                                const x = leftPad + t * chartW;
                                const y = yAtDb(db);
                                if (i === 0)
                                    ctx.moveTo(x, y);
                                else
                                    ctx.lineTo(x, y);
                            }
                            ctx.lineWidth = 2;
                            ctx.strokeStyle = Theme.palette.primary;
                            ctx.stroke();

                            const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands : [];
                            for (let i = 0; i < bands.length; i++) {
                                const band = bands[i];
                                const previewing = eqGraphDragArea.hasDragPreview && eqGraphDragArea.dragBandIndex === i;
                                const freq = previewing ? Number(eqGraphDragArea.dragFreqHz) : Number(band.frequency_hz) || 1000;
                                const gain = previewing ? Number(eqGraphDragArea.dragGainDb) : Number(band.gain_db || 0);
                                const x = xAtFreq(freq);
                                const y = yAtDb(gain + Number(audioMenu.eqPreampDb || 0));
                                const selected = i === audioMenu.eqSelectedBandIndex;

                                ctx.beginPath();
                                ctx.arc(x, y, selected ? 6 : 4.5, 0, 2 * Math.PI);
                                ctx.fillStyle = selected ? Theme.palette.primary : Qt.rgba(Theme.palette.primary.r, Theme.palette.primary.g, Theme.palette.primary.b, 0.7);
                                ctx.fill();

                                ctx.beginPath();
                                ctx.arc(x, y, selected ? 7 : 5.5, 0, 2 * Math.PI);
                                ctx.lineWidth = 1;
                                ctx.strokeStyle = Qt.rgba(1, 1, 1, selected ? 0.7 : 0.35);
                                ctx.stroke();
                            }
                        }

                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                    }

                    MouseArea {
                        id: eqGraphDragArea
                        anchors.fill: parent
                        hoverEnabled: true
                        property bool draggingPoint: false
                        property int dragBandIndex: -1
                        property real dragFreqHz: 0
                        property real dragGainDb: 0
                        property bool hasDragPreview: false

                        function chartBounds() {
                            return {
                                left: 8 + 42,
                                right: width - (8 + 12),
                                top: 8 + 14,
                                bottom: height - (8 + 26)
                            };
                        }

                        function xForFreq(freq, bounds) {
                            const minL = audioMenu.log10(audioMenu.eqGraphMinFreqHz);
                            const maxL = audioMenu.log10(audioMenu.eqGraphMaxFreqHz);
                            const curL = audioMenu.log10(Math.max(audioMenu.eqGraphMinFreqHz, Math.min(audioMenu.eqGraphMaxFreqHz, freq)));
                            const t = (curL - minL) / Math.max(0.0001, maxL - minL);
                            return bounds.left + t * Math.max(1, bounds.right - bounds.left);
                        }

                        function setDragPreviewFromMouse(mx, my) {
                            const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands : [];
                            if (bands.length === 0 || dragBandIndex < 0 || dragBandIndex >= bands.length)
                                return;

                            const b = chartBounds();
                            const x = Math.max(b.left, Math.min(b.right, mx));
                            const y = Math.max(b.top, Math.min(b.bottom, my));

                            const tX = (x - b.left) / Math.max(1, b.right - b.left);
                            const freq = audioMenu.eqGraphFrequencyAt(tX);
                            const tY = 1 - ((y - b.top) / Math.max(1, b.bottom - b.top));
                            const db = audioMenu.eqGraphMinDb + tY * (audioMenu.eqGraphMaxDb - audioMenu.eqGraphMinDb);

                            dragFreqHz = freq;
                            dragGainDb = db - Number(audioMenu.eqPreampDb || 0);
                            hasDragPreview = true;
                            eqGraphCanvas.requestPaint();
                        }

                        function commitDragPreview() {
                            if (!hasDragPreview || dragBandIndex < 0)
                                return;
                            audioMenu.setParametricBandField(dragBandIndex, "frequency_hz", dragFreqHz);
                            audioMenu.setParametricBandField(dragBandIndex, "gain_db", dragGainDb);
                            hasDragPreview = false;
                            dragBandIndex = -1;
                        }

                        onPressed: {
                            const bands = Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands : [];
                            if (bands.length === 0)
                                return;

                            const b = chartBounds();
                            let nearest = -1;
                            let nearestDist = 999999;
                            for (let i = 0; i < bands.length; i++) {
                                const bx = xForFreq(Number(bands[i].frequency_hz) || 1000, b);
                                const byNorm = ((Number(bands[i].gain_db) || 0) + Number(audioMenu.eqPreampDb || 0) - audioMenu.eqGraphMinDb) / Math.max(0.0001, audioMenu.eqGraphMaxDb - audioMenu.eqGraphMinDb);
                                const by = b.top + (1 - byNorm) * Math.max(1, b.bottom - b.top);
                                const dx = mouse.x - bx;
                                const dy = mouse.y - by;
                                const d2 = dx * dx + dy * dy;
                                if (d2 < nearestDist) {
                                    nearestDist = d2;
                                    nearest = i;
                                }
                            }

                            if (nearest >= 0) {
                                audioMenu.eqSelectedBandIndex = nearest;
                                dragBandIndex = nearest;
                                draggingPoint = true;
                                setDragPreviewFromMouse(mouse.x, mouse.y);
                            }
                        }

                        onPositionChanged: {
                            if (draggingPoint)
                                setDragPreviewFromMouse(mouse.x, mouse.y);
                        }

                        onReleased: {
                            commitDragPreview();
                            draggingPoint = false;
                        }

                        onCanceled: {
                            draggingPoint = false;
                            hasDragPreview = false;
                            dragBandIndex = -1;
                            eqGraphCanvas.requestPaint();
                        }
                    }

                    Connections {
                        target: audioMenu
                        function onEqParametricBandsChanged() {
                            eqGraphCanvas.requestPaint();
                        }
                        function onEqPreampDbChanged() {
                            eqGraphCanvas.requestPaint();
                        }
                        function onEqGraphMinDbChanged() {
                            eqGraphCanvas.requestPaint();
                        }
                        function onEqGraphMaxDbChanged() {
                            eqGraphCanvas.requestPaint();
                        }
                        function onEqSelectedBandIndexChanged() {
                            eqGraphCanvas.requestPaint();
                        }
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 12

                    Button {
                        text: "Apply "
                        hoverEnabled: true
                        Layout.fillWidth: true
                        background: Rectangle {
                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgMain
                            radius: 8
                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                            border.width: 1
                        }
                        contentItem: Text {
                            text: parent.text
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        onClicked: audioMenu.applyCurrentEq()
                    }

                    Button {
                        text: "Reset 󰑐"
                        hoverEnabled: true
                        Layout.fillWidth: true
                        background: Rectangle {
                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgMain
                            radius: 8
                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                            border.width: 1
                        }
                        contentItem: Text {
                            text: parent.text
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        onClicked: audioMenu.resetToFlat()
                    }
                }
            }
            ColumnLayout {
                Layout.preferredWidth: 420
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 10

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 10

                    Text {
                        text: "Preamp"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                        Layout.preferredWidth: 56
                    }

                    Slider {
                        id: preampSlider
                        Layout.fillWidth: true
                        from: -12
                        to: 12
                        stepSize: 0.5
                        value: audioMenu.eqPreampDb

                        background: Rectangle {
                            x: preampSlider.leftPadding
                            y: preampSlider.topPadding + preampSlider.availableHeight / 2 - height / 2
                            width: preampSlider.availableWidth
                            height: 4
                            radius: 2
                            color: Theme.palette.bgHover

                            Rectangle {
                                width: preampSlider.visualPosition * parent.width
                                height: parent.height
                                radius: 2
                                color: Theme.palette.primary
                            }
                        }

                        handle: Rectangle {
                            x: preampSlider.leftPadding + preampSlider.visualPosition * (preampSlider.availableWidth - width)
                            y: preampSlider.topPadding + preampSlider.availableHeight / 2 - height / 2
                            implicitWidth: 12
                            implicitHeight: 12
                            radius: 6
                            color: Theme.palette.primary
                        }

                        onMoved: {
                            audioMenu.eqPreampDb = Math.round(value * 10) / 10;
                            audioMenu.currentPresetName = "Custom";
                            audioMenu.isCustomPreset = true;
                        }

                        onValueChanged: {
                            if (!pressed)
                                audioMenu.eqPreampDb = Math.round(value * 10) / 10;
                        }
                    }

                    Text {
                        text: (audioMenu.eqPreampDb > 0 ? "+" : "") + audioMenu.eqPreampDb.toFixed(1) + " dB"
                        color: Theme.palette.textMuted
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        horizontalAlignment: Text.AlignRight
                        Layout.preferredWidth: 58
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    Text {
                        text: "Bands: " + (Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.length : 0)
                        color: Theme.palette.textMuted
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Button {
                        text: "+ Add Band"
                        hoverEnabled: true
                        background: Rectangle {
                            color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
                            border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                            radius: 7
                        }
                        contentItem: Text {
                            text: parent.text
                            color: Theme.palette.textMain
                            font.family: Theme.palette.font
                            font.pixelSize: 10
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        onClicked: audioMenu.addEqBand()
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    radius: 8
                    color: Theme.palette.bgDark
                    border.color: Theme.palette.borderInactive
                    border.width: 1

                    ScrollView {
                        id: bandScrollView
                        anchors.fill: parent
                        clip: false

                        function setFlickEnabled(enabled) {
                            const flick = bandScrollView.contentItem;
                            if (flick && flick.interactive !== undefined)
                                flick.interactive = enabled;
                        }

                        function clamp(value, minValue, maxValue) {
                            return Math.max(minValue, Math.min(maxValue, value));
                        }

                        function scrollToBand(index) {
                            if (index < 0 || index >= bandRepeater.count)
                                return;

                            const bandItem = bandRepeater.itemAt(index);
                            if (!bandItem)
                                return;

                            const topPadding = 8;
                            const bottomPadding = 8;
                            const bandTop = bandItem.y;
                            const bandBottom = bandItem.y + bandItem.height;
                            const viewportHeight = bandScrollView.height;
                            const contentHeight = Math.max(bandColumn.implicitHeight, bandColumn.height);
                            const maxY = Math.max(0, contentHeight - viewportHeight);

                            if (maxY <= 0)
                                return;

                            let currentY = 0;
                            if (bandScrollView.ScrollBar.vertical)
                                currentY = bandScrollView.ScrollBar.vertical.position * maxY;

                            const viewportTop = currentY;
                            const viewportBottom = currentY + viewportHeight;

                            let targetY = currentY;
                            if (bandTop - topPadding < viewportTop) {
                                targetY = Math.max(0, bandTop - topPadding);
                            } else if (bandBottom + bottomPadding > viewportBottom) {
                                targetY = Math.min(maxY, bandBottom + bottomPadding - viewportHeight);
                            }

                            if (Math.abs(targetY - currentY) <= 0.5)
                                return;

                            const targetPos = clamp(targetY / maxY, 0, 1);
                            if (bandScrollView.ScrollBar.vertical) {
                                bandScrollView.ScrollBar.vertical.position = targetPos;
                                return;
                            }

                            // Fallback for styles/platforms where direct scrollbar access isn't available.
                            const flick = bandScrollView.contentItem;
                            if (flick && flick.contentY !== undefined)
                                flick.contentY = targetY;
                        }

                        ColumnLayout {
                            id: bandColumn
                            width: parent.width
                            Layout.fillWidth: true
                            spacing: 10

                            Repeater {
                                id: bandRepeater
                                model: Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands : []

                                delegate: Rectangle {
                                    required property var modelData
                                    required property var index

                                    implicitWidth: bandScrollView.width - 20
                                    implicitHeight: 120
                                    Layout.topMargin: index == 0 ? 10 : 0
                                    Layout.bottomMargin: index == audioMenu.eqParametricBands.length - 1 ? 10 : 0
                                    Layout.leftMargin: 10
                                    Layout.rightMargin: 10
                                    radius: 8
                                    color: Theme.palette.bgWidget
                                    border.color: index === audioMenu.eqSelectedBandIndex ? Theme.palette.borderActive : Theme.palette.borderInactive
                                    border.width: index === audioMenu.eqSelectedBandIndex ? 2 : 1

                                    TapHandler {
                                        onTapped: audioMenu.eqSelectedBandIndex = index
                                    }

                                    Item {
                                        anchors.fill: parent
                                        anchors.margins: 3

                                        ColumnLayout {
                                            id: bandEditor
                                            anchors.fill: parent
                                            anchors.margins: 10
                                            Layout.topMargin: 10
                                            spacing: 5

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: 5

                                                ThemedComboBox {
                                                    id: filterTypeCombo
                                                    Layout.minimumWidth: 110
                                                    Layout.preferredWidth: 120
                                                    boxHeight: 20

                                                    items: audioMenu.eqFilterTypes.map(t => ({
                                                                                                  name: String(t || ""),
                                                                                                  description: String(t || "")
                                                                                              }))
                                                    selectedName: String(modelData.filter_type || "peaking")
                                                    placeholderText: "Filter"
                                                    labelProvider: item => String(item?.name || "")
                                                    onItemChosen: item => audioMenu.setParametricBandField(index, "filter_type", String(item?.name || "peaking"))
                                                }

                                                Text {
                                                    text: audioMenu.formatEqFrequencyLabel(index)
                                                    color: Theme.palette.textMain
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 10
                                                    font.bold: true
                                                    elide: Text.ElideRight
                                                    Layout.minimumWidth: 45
                                                    Layout.fillWidth: true
                                                }

                                                Button {
                                                    text: "Remove"
                                                    enabled: (Array.isArray(audioMenu.eqParametricBands) ? audioMenu.eqParametricBands.length : 0) > 1
                                                    hoverEnabled: true
                                                    Layout.preferredWidth: 70
                                                    background: Rectangle {
                                                        color: parent.enabled ? (parent.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget) : Theme.palette.bgDark
                                                        border.width: 1
                                                        border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                                                        radius: 6
                                                    }
                                                    contentItem: Text {
                                                        text: parent.text
                                                        color: Theme.palette.error
                                                        font.family: Theme.palette.font
                                                        font.pixelSize: 10
                                                        horizontalAlignment: Text.AlignHCenter
                                                        verticalAlignment: Text.AlignVCenter
                                                    }
                                                    onClicked: audioMenu.removeEqBand(index)
                                                }
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true

                                                Text {
                                                    text: "Freq"
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 30
                                                }

                                                TextField {
                                                    id: freqInput
                                                    Layout.fillWidth: true
                                                    text: String(Math.round(Number(modelData.frequency_hz) || 1000))
                                                    color: Theme.palette.textMain
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    horizontalAlignment: TextInput.AlignRight
                                                    rightPadding: 8
                                                    validator: IntValidator {
                                                        bottom: 20
                                                        top: 20000
                                                    }
                                                    inputMethodHints: Qt.ImhDigitsOnly
                                                    selectByMouse: true

                                                    background: Rectangle {
                                                        color: Theme.palette.bgWidget
                                                        border.color: Theme.palette.borderInactive
                                                        border.width: 1
                                                        radius: 6
                                                    }

                                                    onEditingFinished: {
                                                        const parsed = parseInt(text, 10);
                                                        const clamped = Math.max(20, Math.min(20000, isNaN(parsed) ? (Number(modelData.frequency_hz) || 1000) : parsed));
                                                        text = String(clamped);
                                                        audioMenu.setParametricBandField(index, "frequency_hz", clamped);
                                                    }
                                                }

                                                Text {
                                                    text: "Hz"
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 24
                                                    horizontalAlignment: Text.AlignRight
                                                }
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true

                                                Text {
                                                    text: "Gain"
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 30
                                                }

                                                Slider {
                                                    id: gainSlider
                                                    Layout.fillWidth: true
                                                    from: audioMenu.eqMinGain
                                                    to: audioMenu.eqMaxGain
                                                    stepSize: 1
                                                    live: true
                                                    value: Number(modelData.gain_db) || 0
                                                    onPressedChanged: {
                                                        bandScrollView.setFlickEnabled(!pressed);
                                                        if (!pressed)
                                                            audioMenu.setParametricBandField(index, "gain_db", value);
                                                    }

                                                    background: Rectangle {
                                                        x: gainSlider.leftPadding
                                                        y: gainSlider.topPadding + gainSlider.availableHeight / 2 - height / 2
                                                        width: gainSlider.availableWidth
                                                        height: 4
                                                        radius: 2
                                                        color: Theme.palette.bgHover

                                                        Rectangle {
                                                            width: gainSlider.visualPosition * parent.width
                                                            height: parent.height
                                                            radius: 2
                                                            color: Theme.palette.primary
                                                        }
                                                    }

                                                    handle: Rectangle {
                                                        x: gainSlider.leftPadding + gainSlider.visualPosition * (gainSlider.availableWidth - width)
                                                        y: gainSlider.topPadding + gainSlider.availableHeight / 2 - height / 2
                                                        implicitWidth: 12
                                                        implicitHeight: 12
                                                        radius: 6
                                                        color: Theme.palette.primary
                                                    }
                                                }

                                                Text {
                                                    text: ((Number(modelData.gain_db) || 0) > 0 ? "+" : "") + Math.round(Number(modelData.gain_db) || 0) + " dB"
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 40
                                                    horizontalAlignment: Text.AlignRight
                                                }
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true

                                                Text {
                                                    text: "Q"
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 30
                                                }

                                                Slider {
                                                    id: qSlider
                                                    Layout.fillWidth: true
                                                    from: 0.1
                                                    to: 10
                                                    stepSize: 0.1
                                                    live: true
                                                    value: Number(modelData.q) || 0.707
                                                    onPressedChanged: {
                                                        bandScrollView.setFlickEnabled(!pressed);
                                                        if (!pressed)
                                                            audioMenu.setParametricBandField(index, "q", value);
                                                    }

                                                    background: Rectangle {
                                                        x: qSlider.leftPadding
                                                        y: qSlider.topPadding + qSlider.availableHeight / 2 - height / 2
                                                        width: qSlider.availableWidth
                                                        height: 4
                                                        radius: 2
                                                        color: Theme.palette.bgHover

                                                        Rectangle {
                                                            width: qSlider.visualPosition * parent.width
                                                            height: parent.height
                                                            radius: 2
                                                            color: Theme.palette.primary
                                                        }
                                                    }

                                                    handle: Rectangle {
                                                        x: qSlider.leftPadding + qSlider.visualPosition * (qSlider.availableWidth - width)
                                                        y: qSlider.topPadding + qSlider.availableHeight / 2 - height / 2
                                                        implicitWidth: 12
                                                        implicitHeight: 12
                                                        radius: 6
                                                        color: Theme.palette.primary
                                                    }
                                                }

                                                Text {
                                                    text: (Math.round((Number(modelData.q) || 0.707) * 10) / 10).toFixed(1)
                                                    color: Theme.palette.textMuted
                                                    font.family: Theme.palette.font
                                                    font.pixelSize: 11
                                                    Layout.preferredWidth: 40
                                                    horizontalAlignment: Text.AlignRight
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Connections {
                            target: audioMenu

                            function onEqSelectedBandIndexChanged() {
                                Qt.callLater(function () {
                                    bandScrollView.scrollToBand(audioMenu.eqSelectedBandIndex);
                                });
                            }

                            function onEqParametricBandsChanged() {
                                Qt.callLater(function () {
                                    bandScrollView.scrollToBand(audioMenu.eqSelectedBandIndex);
                                });
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.palette.secondary
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 8

            Text {
                text: "Presets"
                color: Theme.palette.textMain
                font.family: Theme.palette.font
                font.pixelSize: 13
                font.bold: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                ThemedComboBox {
                    id: presetCombo
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    boxHeight: 30
                    
                    items: {
                        const names = audioMenu.eqPresets
                            .map(p => String(p.name || ""))
                            .filter(n => n.length > 0);
                        if (!names.includes("Custom"))
                            names.push("Custom");

                        return names.map(n => ({ name: n, description: n }));;
                    }

                    selectedName: audioMenu.defaultInputName
                    placeholderText: "Select Preset"
                    labelProvider: item => String(item?.name || "")
                    onItemChosen: item => {
                        const name = String(item?.name || "");
                        if (name && name !== "Custom")
                            audioMenu.applyPreset(name);
                    }
                }

                Button {
                    text: "Save"
                    hoverEnabled: true
                    background: Rectangle {
                        color: parent.hovered ? Theme.palette.bgHover : Theme.palette.bgMain
                        radius: 8
                        border.width: 1
                        border.color: parent.hovered ? Theme.palette.borderActive : Theme.palette.borderInactive
                    }
                    contentItem: Text {
                        text: parent.text
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    onClicked: audioMenu.savePresetDialog.open()
                }

                Button {
                    text: "Delete"
                    enabled: !audioMenu.isCustomPreset && audioMenu.currentPresetName !== "Flat"
                    hoverEnabled: true
                    background: Rectangle {
                        color: !parent.enabled ? Theme.palette.bgDark : (parent.hovered ? Theme.palette.bgHover : Theme.palette.bgMain)
                        radius: 8
                        border.width: 1
                        border.color: (parent.enabled && parent.hovered) ? Theme.palette.borderActive : Theme.palette.borderInactive
                    }
                    contentItem: Text {
                        text: parent.text
                        color: parent.enabled ? Theme.palette.textMain : Theme.palette.textMuted
                        font.family: Theme.palette.font
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    onClicked: audioMenu.deletePreset(audioMenu.currentPresetName)
                }
            }
        }

        Text {
            Layout.fillWidth: true
            visible: audioMenu.eqStatusMsg.length > 0
            text: audioMenu.eqStatusMsg
            color: audioMenu.eqApplyOk ? Theme.palette.textMuted : Theme.palette.error
            font.family: Theme.palette.font
            font.pixelSize: 10
            wrapMode: Text.WrapAtWordBoundaryOrAnywhere
        }

        Text {
            Layout.fillWidth: true
            visible: audioMenu.eqApplyDryRun
            text: "EQ apply is currently running in dry-run validation mode. Presets save per output, but DSP graph programming is not active yet."
            color: Theme.palette.textMuted
            font.family: Theme.palette.font
            font.pixelSize: 10
            wrapMode: Text.WrapAtWordBoundaryOrAnywhere
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
