import QtQuick
import QtQuick.Layouts
import QtCore
import QtQuick.Dialogs
import QtQuick.Controls
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
    property bool reopenAfterSaveAsDialog: false
    property string statusMessage: ""
    property bool statusError: false
    property var annotationStrokes: []
    property real imageZoom: 1.0
    readonly property real minImageZoom: 1.0
    readonly property real maxImageZoom: 8.0
    property real imagePanX: 0
    property real imagePanY: 0
    property bool panActive: false
    property real panLastSurfaceX: 0
    property real panLastSurfaceY: 0

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
        imageZoom = 1.0;
        imagePanX = 0;
        imagePanY = 0;
        panActive = false;
        paintCanvas.requestPaint();
    }

    function closeViewer() {
        visibleState = false;
        panActive = false;
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

    function clamp(value, minValue, maxValue) {
        return Math.max(minValue, Math.min(maxValue, value));
    }

    function mapPaintToSurface(paintItem, x, y) {
        return paintItem.mapToItem(renderSurface, x, y);
    }

    function mapPaintToImage(paintItem, x, y) {
        const mapped = paintItem.mapToItem(imageDrawSurface, x, y);
        return {
            x: clamp(mapped.x, 0, Math.max(0, imageDrawSurface.width - 1)),
            y: clamp(mapped.y, 0, Math.max(0, imageDrawSurface.height - 1))
        };
    }

    function clampPanForZoom(zoomValue, proposedPanX, proposedPanY) {
        const baseW = Math.max(0, screenshotImage.paintedWidth);
        const baseH = Math.max(0, screenshotImage.paintedHeight);
        const scaledW = baseW * zoomValue;
        const scaledH = baseH * zoomValue;

        let nextPanX = proposedPanX;
        let nextPanY = proposedPanY;

        if (scaledW <= renderSurface.width)
            nextPanX = 0;
        else {
            const limitX = (scaledW - renderSurface.width) / 2;
            nextPanX = clamp(nextPanX, -limitX, limitX);
        }

        if (scaledH <= renderSurface.height)
            nextPanY = 0;
        else {
            const limitY = (scaledH - renderSurface.height) / 2;
            nextPanY = clamp(nextPanY, -limitY, limitY);
        }

        return { x: nextPanX, y: nextPanY };
    }

    function setImageZoom(nextZoom, focusX, focusY) {
        const baseW = Math.max(0, screenshotImage.paintedWidth);
        const baseH = Math.max(0, screenshotImage.paintedHeight);
        if (baseW <= 0 || baseH <= 0)
            return;

        const oldZoom = imageZoom;
        const clampedZoom = clamp(Number(nextZoom || 1), minImageZoom, maxImageZoom);
        const fx = (typeof focusX === "number") ? focusX : (renderSurface.width / 2);
        const fy = (typeof focusY === "number") ? focusY : (renderSurface.height / 2);

        const oldPosX = (renderSurface.width - (baseW * oldZoom)) / 2 + imagePanX;
        const oldPosY = (renderSurface.height - (baseH * oldZoom)) / 2 + imagePanY;
        const contentX = (fx - oldPosX) / Math.max(0.0001, oldZoom);
        const contentY = (fy - oldPosY) / Math.max(0.0001, oldZoom);

        const newPosX = fx - contentX * clampedZoom;
        const newPosY = fy - contentY * clampedZoom;
        const centeredPosX = (renderSurface.width - (baseW * clampedZoom)) / 2;
        const centeredPosY = (renderSurface.height - (baseH * clampedZoom)) / 2;
        const unclampedPanX = newPosX - centeredPosX;
        const unclampedPanY = newPosY - centeredPosY;
        const clampedPan = clampPanForZoom(clampedZoom, unclampedPanX, unclampedPanY);

        imageZoom = clampedZoom;
        imagePanX = clampedPan.x;
        imagePanY = clampedPan.y;
    }

    function startPan(surfaceX, surfaceY) {
        panActive = true;
        panLastSurfaceX = surfaceX;
        panLastSurfaceY = surfaceY;
    }

    function updatePan(surfaceX, surfaceY) {
        if (!panActive)
            return;

        const dx = surfaceX - panLastSurfaceX;
        const dy = surfaceY - panLastSurfaceY;
        panLastSurfaceX = surfaceX;
        panLastSurfaceY = surfaceY;

        const clampedPan = clampPanForZoom(imageZoom, imagePanX + dx, imagePanY + dy);
        imagePanX = clampedPan.x;
        imagePanY = clampedPan.y;
    }

    function stopPan() {
        panActive = false;
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

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "Pen"
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                }

                Slider {
                    id: penSizeSlider
                    Layout.preferredWidth: 120
                    from: 1
                    to: 16
                    stepSize: 1
                    value: viewer.penSize
                    onMoved: viewer.penSize = Math.round(value)
                }

                Binding {
                    target: penSizeSlider
                    property: "value"
                    value: viewer.penSize
                    when: !penSizeSlider.pressed
                }

                Text {
                    text: String(viewer.penSize)
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.preferredWidth: 18
                }

                Text {
                    text: "Zoom"
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.leftMargin: 8
                }

                Slider {
                    id: zoomSlider
                    Layout.preferredWidth: 170
                    from: viewer.minImageZoom
                    to: viewer.maxImageZoom
                    stepSize: 0.1
                    value: viewer.minImageZoom
                    enabled: screenshotImage.status === Image.Ready
                    onMoved: viewer.setImageZoom(value, renderSurface.width / 2, renderSurface.height / 2)
                }

                Binding {
                    target: zoomSlider
                    property: "value"
                    value: viewer.imageZoom
                    when: !zoomSlider.pressed
                }

                Text {
                    text: Math.round(viewer.imageZoom * 100) + "%"
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.preferredWidth: 42
                }
            }

            Item {
                id: renderSurface
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true

                onWidthChanged: viewer.setImageZoom(viewer.imageZoom, width / 2, height / 2)
                onHeightChanged: viewer.setImageZoom(viewer.imageZoom, width / 2, height / 2)

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
                    x: (renderSurface.width - (width * viewer.imageZoom)) / 2 + viewer.imagePanX
                    y: (renderSurface.height - (height * viewer.imageZoom)) / 2 + viewer.imagePanY
                    width: screenshotImage.paintedWidth
                    height: screenshotImage.paintedHeight
                    clip: true
                    scale: viewer.imageZoom
                    transformOrigin: Item.TopLeft

                    Image {
                        id: drawImageBase
                        anchors.fill: parent
                        source: screenshotImage.source
                        fillMode: Image.Stretch
                        smooth: false
                        cache: false
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
                            id: paintInputArea
                            anchors.fill: parent
                            cursorShape: Qt.CrossCursor
                            acceptedButtons: Qt.LeftButton | Qt.MiddleButton
                            hoverEnabled: true

                            onWheel: function(wheel) {
                                if (!(wheel.modifiers & Qt.ControlModifier)) {
                                    wheel.accepted = false;
                                    return;
                                }

                                const dy = wheel.angleDelta.y !== 0 ? wheel.angleDelta.y : wheel.pixelDelta.y;
                                if (dy === 0) {
                                    wheel.accepted = true;
                                    return;
                                }

                                const surfacePos = viewer.mapPaintToSurface(paintInputArea, wheel.x, wheel.y);
                                const zoomStep = dy > 0 ? 0.15 : -0.15;
                                viewer.setImageZoom(viewer.imageZoom + zoomStep, surfacePos.x, surfacePos.y);
                                wheel.accepted = true;
                            }

                            onPressed: function(mouse) {
                                const surfacePos = viewer.mapPaintToSurface(paintInputArea, mouse.x, mouse.y);
                                const imagePos = viewer.mapPaintToImage(paintInputArea, mouse.x, mouse.y);

                                if (mouse.button === Qt.MiddleButton || (mouse.modifiers & Qt.ControlModifier)) {
                                    viewer.startPan(surfacePos.x, surfacePos.y);
                                    paintCanvas.currentStrokeIndex = -1;
                                    return;
                                }

                                const strokes = viewer.annotationStrokes.slice();
                                const stroke = {
                                    color: viewer.colorToHex(viewer.annotationColor),
                                    size: viewer.penSize,
                                    points: [{ x: imagePos.x, y: imagePos.y }]
                                };
                                strokes.push(stroke);
                                viewer.annotationStrokes = strokes;
                                paintCanvas.currentStrokeIndex = strokes.length - 1;
                                paintCanvas.requestPaint();
                            }

                            onPositionChanged: function(mouse) {
                                const surfacePos = viewer.mapPaintToSurface(paintInputArea, mouse.x, mouse.y);
                                const imagePos = viewer.mapPaintToImage(paintInputArea, mouse.x, mouse.y);

                                if (viewer.panActive) {
                                    viewer.updatePan(surfacePos.x, surfacePos.y);
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

                                stroke.points.push({ x: imagePos.x, y: imagePos.y });
                                strokes[idx] = stroke;
                                viewer.annotationStrokes = strokes;
                                paintCanvas.requestPaint();
                            }

                            onReleased: {
                                if (viewer.panActive) {
                                    viewer.stopPan();
                                    paintCanvas.currentStrokeIndex = -1;
                                    return;
                                }
                                paintCanvas.currentStrokeIndex = -1;
                            }

                            onCanceled: {
                                viewer.stopPan();
                                paintCanvas.currentStrokeIndex = -1;
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
