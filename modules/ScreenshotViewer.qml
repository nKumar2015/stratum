import QtQuick
import QtQuick.Layouts
import QtCore
import QtQuick.Dialogs
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: viewer

    property bool visibleState: false
    property string captureMode: "window"
    property string sourcePath: ""
    property color annotationColor: "#ff3b30"
    property int penSize: 3
    property bool isWorking: false
    property bool colorPickerActive: false
    property real pickerHoverX: 0
    property real pickerHoverY: 0
    property string pickedColorHex: ""
    property bool reopenAfterSaveAsDialog: false
    property string statusMessage: ""
    property bool statusError: false
    property var annotationStrokes: []

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    color: "#99000000"
    visible: visibleState
    exclusiveZone: -1

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    function toFileUrl(path) {
        const value = String(path || "");
        if (!value)
            return "";
        if (value.startsWith("file://"))
            return value;
        return "file://" + encodeURI(value);
    }

    function toLocalPath(pathOrUrl) {
        let value = String(pathOrUrl || "").trim();
        if (!value)
            return "";

        if (value.startsWith("file://")) {
            value = value.substring("file://".length);
            value = "/" + value.replace(/^\/+/, "");
        }

        try {
            value = decodeURIComponent(value);
        } catch (_error) {
        }

        return value;
    }

    function openViewer(imagePath, mode) {
        sourcePath = toLocalPath(imagePath);
        captureMode = String(mode || "window");
        visibleState = sourcePath.length > 0;
        statusMessage = "";
        statusError = false;
        annotationStrokes = [];
        paintCanvas.requestPaint();
    }

    function closeViewer() {
        visibleState = false;
        colorPickerActive = false;
        statusMessage = "";
        statusError = false;
        isWorking = false;
    }

    function showStatus(message, isError) {
        statusMessage = String(message || "");
        statusError = !!isError;
        if (!statusMessage)
            return;
        clearStatusTimer.restart();
    }

    function colorToHex(c) {
        const r = Math.round(c.r * 255).toString(16).padStart(2, "0");
        const g = Math.round(c.g * 255).toString(16).padStart(2, "0");
        const b = Math.round(c.b * 255).toString(16).padStart(2, "0");
        return "#" + r + g + b;
    }

    function clearAnnotations() {
        annotationStrokes = [];
        paintCanvas.requestPaint();
    }

    function hasAnnotations() {
        return (annotationStrokes || []).length > 0;
    }

    function startPostAction(action) {
        startPostActionWithTarget(action, "");
    }

    function startPostActionWithTarget(action, targetPath) {
        if (isWorking)
            return;
        if (!sourcePath) {
            showStatus("No image loaded", true);
            return;
        }

        isWorking = true;
        if (hasAnnotations()) {
            const runtimeDir = StandardPaths.writableLocation(StandardPaths.RuntimeLocation) || "/tmp";
            const composedPath = runtimeDir + "/quickshell-screenshot-viewer-" + Date.now() + ".png";
            imageDrawSurface.grabToImage(function(result) {
                const saved = result.saveToFile(composedPath);
                if (!saved) {
                    viewer.isWorking = false;
                    viewer.showStatus("Failed to render annotated image", true);
                    return;
                }

                const args = ["sh", Quickshell.shellDir + "/scripts/screenshot_viewer.sh", action, composedPath];
                const normalizedTarget = viewer.toLocalPath(targetPath);
                if (normalizedTarget.length > 0)
                    args.push(normalizedTarget);
                postProc.command = args;
                postProc.running = true;
            });
            return;
        }

        const args = ["sh", Quickshell.shellDir + "/scripts/screenshot_viewer.sh", action, toLocalPath(sourcePath)];
        const normalizedTarget = toLocalPath(targetPath);
        if (normalizedTarget.length > 0)
            args.push(normalizedTarget);
        postProc.command = args;
        postProc.running = true;
    }

    function startSaveAs() {
        if (isWorking)
            return;
        if (!sourcePath) {
            showStatus("No image loaded", true);
            return;
        }

        const picturesDir = StandardPaths.writableLocation(StandardPaths.PicturesLocation);
        const baseDir = (picturesDir && picturesDir.length > 0) ? picturesDir + "/Screenshots" : "/tmp";
        const stamp = new Date();
        const pad = n => String(n).padStart(2, "0");
        const name = "Screenshot-" + stamp.getFullYear() + pad(stamp.getMonth() + 1) + pad(stamp.getDate()) + "-" + pad(stamp.getHours()) + pad(stamp.getMinutes()) + pad(stamp.getSeconds()) + ".png";
        const baseDirUrl = "file://" + encodeURI(baseDir);
        reopenAfterSaveAsDialog = viewer.visibleState;
        viewer.visibleState = false;
        saveAsDialog.currentFolder = baseDirUrl;
        saveAsDialog.selectedFile = baseDirUrl + "/" + encodeURIComponent(name);
        saveAsDialog.open();
    }

    function toggleColorPicker() {
        if (isWorking)
            return;
        colorPickerActive = !colorPickerActive;
        if (colorPickerActive) {
            showStatus("Click image to pick color", false);
            pickerHoverX = 0;
            pickerHoverY = 0;
            zoomCanvas.requestPaint();
        }
    }

    function clamp(value, minValue, maxValue) {
        return Math.max(minValue, Math.min(maxValue, value));
    }

    function pickColorAt(x, y) {
        colorSampleCanvas.requestPaint();
        const sx = Math.round(clamp(x, 0, Math.max(0, colorSampleCanvas.width - 1)));
        const sy = Math.round(clamp(y, 0, Math.max(0, colorSampleCanvas.height - 1)));
        const ctx = colorSampleCanvas.getContext("2d");
        if (!ctx)
            return;

        const px = ctx.getImageData(sx, sy, 1, 1).data;
        if (!px || px.length < 3)
            return;

        const toHex = n => Number(n || 0).toString(16).padStart(2, "0");
        const hex = "#" + toHex(px[0]) + toHex(px[1]) + toHex(px[2]);
        pickedColorHex = hex;
        colorCopyProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_viewer.sh", "copy-text", hex];
        colorCopyProc.running = true;
    }

    Timer {
        id: clearStatusTimer
        interval: 2600
        repeat: false
        onTriggered: {
            viewer.statusMessage = "";
            viewer.statusError = false;
        }
    }

    Connections {
        target: GlobalState

        function onScreenshotViewerOpenRequested(imagePath, mode) {
            viewer.openViewer(imagePath, mode);
        }
    }

    Process {
        id: postProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                viewer.isWorking = false;
                const result = this.text.trim();
                if (!result) {
                    viewer.showStatus("Action failed: empty response", true);
                    return;
                }

                if (result.startsWith("__ERROR__|")) {
                    const message = result.substring("__ERROR__|".length);
                    viewer.showStatus(message || "Action failed", true);
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Viewer action failed",
                        body: message || "Unknown error",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                const parts = result.split("|");
                if (parts[0] !== "ok") {
                    viewer.showStatus("Action failed", true);
                    return;
                }

                const action = parts[1] || "";
                const payload = parts[2] || "";
                if (action === "copy") {
                    viewer.showStatus("Copied to clipboard", false);
                    return;
                }

                if (action === "save") {
                    viewer.showStatus("Saved screenshot", false);
                    return;
                }

                if (action === "save-as") {
                    viewer.showStatus("Saved screenshot", false);
                    return;
                }

                viewer.showStatus("Done", false);
            }
        }
    }

    Process {
        id: colorCopyProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (!result) {
                    viewer.showStatus("Failed to copy color", true);
                    return;
                }

                if (result.startsWith("__ERROR__|")) {
                    const message = result.substring("__ERROR__|".length);
                    viewer.showStatus(message || "Failed to copy color", true);
                    return;
                }

                viewer.showStatus("Copied color " + viewer.pickedColorHex, false);
            }
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (viewer.visibleState)
                viewer.closeViewer();
        }
    }

    MouseArea {
        anchors.fill: parent
        enabled: viewer.visibleState
        onClicked: viewer.closeViewer()
    }

    Rectangle {
        id: frame
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1320)
        height: Math.min(parent.height - 80, 860)
        radius: 14
        color: Theme.background
        border.width: 1
        border.color: Theme.grey
        visible: viewer.visibleState

        MouseArea {
            anchors.fill: parent
            onClicked: {
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 12
            spacing: 10

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Text {
                    text: "Screenshot Viewer"
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 14
                    font.bold: true
                }

                Text {
                    text: "Mode: " + viewer.captureMode
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.leftMargin: 8
                }

                Item {
                    Layout.fillWidth: true
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: Theme.black
                    border.width: 1
                    border.color: Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: "󰃢"
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.clearAnnotations()
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: viewer.colorPickerActive ? Theme.activeWs : Theme.black
                    border.width: 1
                    border.color: viewer.colorPickerActive ? Theme.activeWs : Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: ""
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.toggleColorPicker()
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: viewer.isWorking ? Theme.hover : Theme.activeWs
                    border.width: 1
                    border.color: Theme.activeWs

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆏"
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startPostAction("copy")
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: viewer.isWorking ? Theme.hover : Theme.activeWs
                    border.width: 1
                    border.color: Theme.activeWs

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆓"
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startPostAction("save")
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: viewer.isWorking ? Theme.hover : Theme.activeWs
                    border.width: 1
                    border.color: Theme.activeWs

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰉋"
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startSaveAs()
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: Theme.black
                    border.width: 1
                    border.color: Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.closeViewer()
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Text {
                    text: "Annotation color"
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                }

                Repeater {
                    model: ["#ff3b30", "#34c759", "#0a84ff", "#ffd60a", "#ffffff", "#000000"]
                    delegate: Rectangle {
                        required property var modelData
                        Layout.preferredWidth: 20
                        Layout.preferredHeight: 20
                        radius: 10
                        color: modelData
                        border.width: viewer.annotationColor == modelData ? 2 : 1
                        border.color: viewer.annotationColor == modelData ? Theme.activeWs : Theme.grey

                        MouseArea {
                            anchors.fill: parent
                            onClicked: viewer.annotationColor = modelData
                        }
                    }
                }

                Text {
                    text: "Picked: " + viewer.colorToHex(viewer.annotationColor)
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.leftMargin: 8
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "Pen"
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                }

                Repeater {
                    model: [2, 3, 5, 8]
                    delegate: Rectangle {
                        required property var modelData
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 20
                        radius: 5
                        color: viewer.penSize === modelData ? Theme.activeWs : Theme.black
                        border.width: 1
                        border.color: Theme.grey

                        Text {
                            anchors.centerIn: parent
                            text: String(modelData)
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 10
                        }

                        MouseArea {
                            anchors.fill: parent
                            onClicked: viewer.penSize = modelData
                        }
                    }
                }
            }

            Item {
                id: renderSurface
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true

                Image {
                    id: screenshotImage
                    anchors.fill: parent
                    source: viewer.toFileUrl(viewer.sourcePath)
                    fillMode: Image.PreserveAspectFit
                    smooth: true
                    cache: false
                }

                Item {
                    id: imageDrawSurface
                    visible: screenshotImage.status === Image.Ready
                    x: screenshotImage.x + (screenshotImage.width - screenshotImage.paintedWidth) / 2
                    y: screenshotImage.y + (screenshotImage.height - screenshotImage.paintedHeight) / 2
                    width: screenshotImage.paintedWidth
                    height: screenshotImage.paintedHeight
                    clip: true

                    Image {
                        id: drawImageBase
                        anchors.fill: parent
                        source: screenshotImage.source
                        fillMode: Image.Stretch
                        smooth: true
                        cache: false

                        onStatusChanged: colorSampleCanvas.requestPaint()
                        onSourceChanged: colorSampleCanvas.requestPaint()
                    }

                    Canvas {
                        id: paintCanvas
                        anchors.fill: parent

                        onPaint: {
                            const ctx = getContext("2d");
                            ctx.clearRect(0, 0, paintCanvas.width, paintCanvas.height);
                            const strokes = viewer.annotationStrokes || [];
                            for (let i = 0; i < strokes.length; i++) {
                                const stroke = strokes[i];
                                const points = stroke.points || [];
                                if (points.length < 2)
                                    continue;

                                ctx.lineJoin = "round";
                                ctx.lineCap = "round";
                                ctx.strokeStyle = stroke.color;
                                ctx.lineWidth = stroke.size;
                                ctx.beginPath();
                                ctx.moveTo(points[0].x, points[0].y);
                                for (let p = 1; p < points.length; p++)
                                    ctx.lineTo(points[p].x, points[p].y);
                                ctx.stroke();
                            }
                        }

                        property int currentStrokeIndex: -1

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.CrossCursor

                            onPressed: function(mouse) {
                                if (viewer.colorPickerActive) {
                                    viewer.pickerHoverX = mouse.x;
                                    viewer.pickerHoverY = mouse.y;
                                    zoomCanvas.requestPaint();
                                    viewer.pickColorAt(mouse.x, mouse.y);
                                    viewer.colorPickerActive = false;
                                    return;
                                }

                                const strokes = viewer.annotationStrokes.slice();
                                const stroke = {
                                    color: viewer.colorToHex(viewer.annotationColor),
                                    size: viewer.penSize,
                                    points: [{ x: mouse.x, y: mouse.y }]
                                };
                                strokes.push(stroke);
                                viewer.annotationStrokes = strokes;
                                paintCanvas.currentStrokeIndex = strokes.length - 1;
                                paintCanvas.requestPaint();
                            }

                            onPositionChanged: function(mouse) {
                                if (viewer.colorPickerActive) {
                                    viewer.pickerHoverX = mouse.x;
                                    viewer.pickerHoverY = mouse.y;
                                    zoomCanvas.requestPaint();
                                    return;
                                }

                                if (!(mouse.buttons & Qt.LeftButton))
                                    return;

                                const idx = paintCanvas.currentStrokeIndex;
                                if (idx < 0)
                                    return;

                                const strokes = viewer.annotationStrokes.slice();
                                const stroke = strokes[idx];
                                if (!stroke || !stroke.points)
                                    return;

                                stroke.points.push({ x: mouse.x, y: mouse.y });
                                strokes[idx] = stroke;
                                viewer.annotationStrokes = strokes;
                                paintCanvas.requestPaint();
                            }

                            onReleased: {
                                paintCanvas.currentStrokeIndex = -1;
                            }

                            hoverEnabled: true
                        }
                    }

                    Canvas {
                        id: colorSampleCanvas
                        anchors.fill: parent
                        visible: false

                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()

                        onPaint: {
                            const ctx = getContext("2d");
                            ctx.clearRect(0, 0, width, height);
                            ctx.drawImage(drawImageBase, 0, 0, width, height);
                            zoomCanvas.requestPaint();
                        }
                    }

                    Rectangle {
                        id: zoomLens
                        visible: viewer.colorPickerActive
                        width: 96
                        height: 96
                        radius: 8
                        border.width: 1
                        border.color: Theme.activeWs
                        color: Theme.black
                        x: viewer.clamp(viewer.pickerHoverX + 18, 0, Math.max(0, imageDrawSurface.width - width))
                        y: viewer.clamp(viewer.pickerHoverY + 18, 0, Math.max(0, imageDrawSurface.height - height))

                        Canvas {
                            id: zoomCanvas
                            anchors.fill: parent
                            onPaint: {
                                const ctx = getContext("2d");
                                ctx.clearRect(0, 0, width, height);
                                const sampleCtx = colorSampleCanvas.getContext("2d");
                                if (!sampleCtx)
                                    return;

                                const sampleSize = 13;
                                const half = Math.floor(sampleSize / 2);
                                const centerX = Math.round(viewer.pickerHoverX);
                                const centerY = Math.round(viewer.pickerHoverY);
                                const sx = viewer.clamp(centerX - half, 0, Math.max(0, colorSampleCanvas.width - sampleSize));
                                const sy = viewer.clamp(centerY - half, 0, Math.max(0, colorSampleCanvas.height - sampleSize));
                                const data = sampleCtx.getImageData(sx, sy, sampleSize, sampleSize).data;
                                const cellW = width / sampleSize;
                                const cellH = height / sampleSize;

                                for (let y = 0; y < sampleSize; y++) {
                                    for (let x = 0; x < sampleSize; x++) {
                                        const idx = (y * sampleSize + x) * 4;
                                        ctx.fillStyle = "rgba(" + data[idx] + "," + data[idx + 1] + "," + data[idx + 2] + "," + (data[idx + 3] / 255) + ")";
                                        ctx.fillRect(x * cellW, y * cellH, cellW + 0.5, cellH + 0.5);
                                    }
                                }

                                ctx.strokeStyle = "#ffffff";
                                ctx.lineWidth = 1;
                                ctx.beginPath();
                                ctx.moveTo(width / 2, 0);
                                ctx.lineTo(width / 2, height);
                                ctx.moveTo(0, height / 2);
                                ctx.lineTo(width, height / 2);
                                ctx.stroke();
                            }
                        }
                    }
                }
            }

            Rectangle {
                visible: viewer.statusMessage.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: 28
                radius: 6
                color: viewer.statusError ? "#662222" : "#1c2c1c"
                border.width: 1
                border.color: viewer.statusError ? "#aa4444" : "#3f8f3f"

                Text {
                    anchors.centerIn: parent
                    text: viewer.statusMessage
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 11
                    font.bold: true
                }
            }
        }
    }

    FileDialog {
        id: saveAsDialog
        title: "Save Screenshot As"
        fileMode: FileDialog.SaveFile
        nameFilters: ["PNG Image (*.png)"]
        defaultSuffix: "png"

        onAccepted: {
            if (viewer.reopenAfterSaveAsDialog)
                viewer.visibleState = true;
            viewer.reopenAfterSaveAsDialog = false;
            const chosenPath = viewer.toLocalPath(String(selectedFile || ""));
            if (!chosenPath) {
                viewer.showStatus("Save As cancelled", true);
                return;
            }
            viewer.startPostActionWithTarget("save-to", chosenPath);
        }

        onRejected: {
            if (viewer.reopenAfterSaveAsDialog)
                viewer.visibleState = true;
            viewer.reopenAfterSaveAsDialog = false;
        }
    }
}
