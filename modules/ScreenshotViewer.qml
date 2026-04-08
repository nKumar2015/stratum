pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtCore
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
import Quickshell.Io

import "../globals"

PanelWindow {
    id: viewer

    property bool visibleState: false
    property string captureMode: "window"
    property string sourcePath: ""
    property color annotationColor: "#ff3b30"
    property int penSize: 3
    property bool isWorking: false
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
    property var resolvedScreen: null
    property string postActionTempPath: ""
    property string pendingAnnotatedAction: ""
    property string pendingAnnotatedTargetPath: ""
    property bool annotatedExportPending: false
    property int annotatedExportRetries: 0
    property real annotatedExportWidth: 0
    property real annotatedExportHeight: 0
    readonly property bool portalBusy: portalSaveAsProc.running
    readonly property bool viewerInteractive: visibleState && !portalBusy
    readonly property bool saveAsVerboseLogs: false
    readonly property int portalSaveAsTimeoutMs: 35000
    readonly property int annotatedExportPollMs: 16
    readonly property int annotatedExportMaxRetries: 60

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    screen: resolvedScreen

    color: "#00000000"
    visible: viewerInteractive
    exclusiveZone: -1

    // Keep the viewer above normal UI during editing and Save As flow.
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: viewerInteractive ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    ScreenshotWorkflowHelper {
        id: workflow
        viewer: viewer
        annotationExportSurface: annotationExportSurface
        annotationExportCanvas: annotationExportCanvas
    }

    ScreenshotCanvasHelper {
        id: canvasOps
        viewer: viewer
        renderSurface: renderSurface
        screenshotImage: screenshotImage
        imageDrawSurface: imageDrawSurface
    }

    function screenForMonitorName(name) {
        const wanted = String(name || "").trim();
        const screens = Quickshell.screens || [];
        if (!screens.length)
            return null;

        if (!wanted)
            return screens[0];

        for (let i = 0; i < screens.length; i++) {
            const mon = Hyprland.monitorFor(screens[i]);
            if (String(mon?.name || "") === wanted)
                return screens[i];
        }

        return screens[0];
    }

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
        } catch (_error) {}

        return value;
    }

    function openViewer(imagePath, mode) {
        resolvedScreen = screenForMonitorName(GlobalState.popupMonitorName);
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
        pendingAnnotatedAction = "";
        pendingAnnotatedTargetPath = "";
        annotatedExportPending = false;
        if (!postProc.running)
            cleanupPostActionTempPath();
        paintCanvas.requestPaint();
    }

    function closeViewer() {
        visibleState = false;
        panActive = false;
        statusMessage = "";
        statusError = false;
        isWorking = false;
        annotatedExportPending = false;
        pendingAnnotatedAction = "";
        pendingAnnotatedTargetPath = "";
        if (!postProc.running)
            cleanupPostActionTempPath();
    }

    function sourcePixelSize() {
        const preferredW = Math.round(Number(screenshotImage.sourceSize.width || drawImageBase.sourceSize.width || 0));
        const preferredH = Math.round(Number(screenshotImage.sourceSize.height || drawImageBase.sourceSize.height || 0));
        if (preferredW > 0 && preferredH > 0)
            return {
                w: preferredW,
                h: preferredH
            };

        const fallbackW = Math.max(1, Math.round(Number(imageDrawSurface.width || 0)));
        const fallbackH = Math.max(1, Math.round(Number(imageDrawSurface.height || 0)));
        if (fallbackW > 0 && fallbackH > 0)
            return {
                w: fallbackW,
                h: fallbackH
            };

        return null;
    }

    function cleanupPostActionTempPath() {
        const tempPath = toLocalPath(postActionTempPath);
        if (!tempPath)
            return;

        postActionTempPath = "";
        tempCleanupProc.command = ["rm", "-f", tempPath];
        tempCleanupProc.running = true;
    }

    function runPostAction(action, imagePath, targetPath) {
        const args = ["stratum-cli", "screenshot-post", String(action || ""), toLocalPath(imagePath)];
        const normalizedTarget = toLocalPath(targetPath);
        if (normalizedTarget.length > 0)
            args.push(normalizedTarget);
        postProc.command = args;
        postProc.running = true;
    }

    function startAnnotatedPostAction(action, targetPath) {
        const sourceSize = sourcePixelSize();
        if (!sourceSize) {
            isWorking = false;
            showStatus("Failed to prepare annotated export", true);
            return;
        }

        const runtimeDir = StandardPaths.writableLocation(StandardPaths.RuntimeLocation) || "/tmp";
        cleanupPostActionTempPath();
        postActionTempPath = runtimeDir + "/quickshell-screenshot-viewer-" + Date.now() + ".png";
        pendingAnnotatedAction = String(action || "");
        pendingAnnotatedTargetPath = toLocalPath(targetPath);
        annotatedExportWidth = sourceSize.w;
        annotatedExportHeight = sourceSize.h;
        annotatedExportRetries = 0;
        annotatedExportPending = true;
        annotationExportCanvas.requestPaint();
        annotatedExportGrabTimer.restart();
    }

    function showStatus(message, isError) {
        statusMessage = String(message || "");
        statusError = !!isError;
        if (!statusMessage)
            return;
        clearStatusTimer.restart();
    }

    function logSaveAs(message) {
        if (!saveAsVerboseLogs)
            return;
        console.log("[QS][Screenshot][SaveAs] " + String(message || ""));
    }

    function logSaveAsPayload(streamName, payload) {
        const raw = String(payload || "");
        if (!raw.length) {
            logSaveAs(streamName + " <empty>");
            return;
        }

        const lines = raw.split(/\r?\n/);
        for (let i = 0; i < lines.length; i++) {
            if (!lines[i].length)
                continue;
            logSaveAs(streamName + " " + lines[i]);
        }
    }

    function statusForPostAction(action) {
        const key = String(action || "");
        if (key === "copy")
            return "Copied to clipboard";
        if (key === "save" || key === "save-as" || key === "save-to")
            return "Saved screenshot";
        if (key === "copy-text")
            return "Copied text to clipboard";
        return "Done";
    }

    function saveAsError(message) {
        const detail = String(message || "").trim();
        if (!detail)
            return "Save As failed";
        return "Save As failed: " + detail;
    }

    function parseCliJson(raw) {
        const text = String(raw || "").trim();
        if (!text.length)
            return null;

        try {
            return JSON.parse(text);
        } catch (_error) {
            return null;
        }
    }

    function colorToHex(c) {
        return canvasOps.colorToHex(c);
    }

    function clearAnnotations() {
        canvasOps.clearAnnotations();
        paintCanvas.requestPaint();
    }

    function undoLastAnnotation() {
        canvasOps.undoLastAnnotation();
        paintCanvas.currentStrokeIndex = -1;
        paintCanvas.requestPaint();
    }

    function hasAnnotations() {
        return canvasOps.hasAnnotations();
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
            startAnnotatedPostAction(action, targetPath);
            return;
        }

        runPostAction(action, sourcePath, targetPath);
    }

    function startSaveAs() {
        if (isWorking || portalSaveAsProc.running)
            return;
        if (!sourcePath) {
            showStatus("No image loaded", true);
            return;
        }

        const stamp = new Date();
        const pad = n => String(n).padStart(2, "0");
        const name = "Screenshot-" + stamp.getFullYear() + pad(stamp.getMonth() + 1) + pad(stamp.getDate()) + "-" + pad(stamp.getHours()) + pad(stamp.getMinutes()) + pad(stamp.getSeconds()) + ".png";
        portalSaveAsGuard.restart();
        portalSaveAsProc.command = ["stratum-cli", "portal-save-file", "Save Screenshot As", name];
        logSaveAs("launch command=" + JSON.stringify(portalSaveAsProc.command));
        portalSaveAsProc.running = true;
    }

    function clamp(value, minValue, maxValue) {
        return canvasOps.clamp(value, minValue, maxValue);
    }

    function mapPaintToSurface(paintItem, x, y) {
        return canvasOps.mapPaintToSurface(paintItem, x, y);
    }

    function mapPaintToImage(paintItem, x, y) {
        return canvasOps.mapPaintToImage(paintItem, x, y);
    }

    function clampPanForZoom(zoomValue, proposedPanX, proposedPanY) {
        return canvasOps.clampPanForZoom(zoomValue, proposedPanX, proposedPanY);
    }

    function setImageZoom(nextZoom, focusX, focusY) {
        canvasOps.setImageZoom(nextZoom, focusX, focusY);
    }

    function startPan(surfaceX, surfaceY) {
        canvasOps.startPan(surfaceX, surfaceY);
    }

    function updatePan(surfaceX, surfaceY) {
        canvasOps.updatePan(surfaceX, surfaceY);
    }

    function stopPan() {
        canvasOps.stopPan();
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

    Timer {
        id: portalSaveAsGuard
        interval: viewer.portalSaveAsTimeoutMs
        repeat: false
        onTriggered: {
            if (!portalSaveAsProc.running)
                return;

            viewer.logSaveAs("guard timeout: process still running after " + interval + "ms");
            portalSaveAsProc.running = false;
            viewer.showStatus(viewer.saveAsError("portal request timed out"), true);
        }
    }

    Timer {
        id: annotatedExportGrabTimer
        interval: viewer.annotatedExportPollMs
        repeat: false
        onTriggered: {
            if (!viewer.annotatedExportPending)
                return;

            if (annotationExportBase.status === Image.Error) {
                workflow.failAnnotatedExport("Failed to load source image for export", true);
                return;
            }

            if (annotationExportBase.status !== Image.Ready) {
                viewer.annotatedExportRetries = viewer.annotatedExportRetries + 1;
                if (viewer.annotatedExportRetries <= viewer.annotatedExportMaxRetries) {
                    annotatedExportGrabTimer.restart();
                    return;
                }

                workflow.failAnnotatedExport("Timed out preparing annotated export", true);
                return;
            }

            workflow.grabAnnotatedExportImage();
        }
    }

    Connections {
        target: GlobalState

        function onScreenshotViewerOpenRequested(imagePath, mode) {
            viewer.openViewer(imagePath, mode);
        }
    }

    Process {
        id: tempCleanupProc
        running: false
    }

    Process {
        id: postProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                viewer.isWorking = false;
                viewer.cleanupPostActionTempPath();
                workflow.handlePostActionResult(this.text);
            }
        }
    }

    Process {
        id: portalSaveAsProc
        running: false
        stderr: StdioCollector {
            onStreamFinished: {
                viewer.logSaveAsPayload("stderr", this.text);
            }
        }
        stdout: StdioCollector {
            onStreamFinished: {
                portalSaveAsGuard.stop();
                const rawResult = String(this.text || "");
                viewer.logSaveAsPayload("stdout", rawResult);
                workflow.handlePortalSaveAsResult(rawResult);
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

    Shortcut {
        sequence: "Ctrl+Z"
        onActivated: {
            if (viewer.visibleState && !viewer.isWorking)
                viewer.undoLastAnnotation();
        }
    }

    MouseArea {
        anchors.fill: parent
        enabled: viewer.viewerInteractive
        onClicked: viewer.closeViewer()
    }

    Rectangle {
        id: frame
        anchors.centerIn: parent
        width: Math.min(parent.width - 80, 1320)
        height: Math.min(parent.height - 80, 860)
        radius: 14
        color: Theme.palette.bgMain
        visible: viewer.viewerInteractive

        MouseArea {
            anchors.fill: parent
            onClicked: {}
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
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 14
                    font.bold: true
                }

                Text {
                    text: "Mode: " + viewer.captureMode
                    color: Theme.palette.secondary
                    font.family: Theme.palette.font
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
                    color: clearMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: "󰃢"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        id: clearMouse
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.clearAnnotations()
                        hoverEnabled: true
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: copyMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆏"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        id: copyMouse
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startPostAction("copy")
                        hoverEnabled: true
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: saveMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆓"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        id: saveMouse
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startPostAction("save")
                        hoverEnabled: true
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: saveAsMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰉋"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        id: saveAsMouse
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.startSaveAs()
                        hoverEnabled: true
                    }
                }

                Rectangle {
                    Layout.preferredHeight: 32
                    Layout.preferredWidth: 42
                    radius: 8
                    color: closeMouse.containsMouse ? Theme.palette.bgHover : Theme.palette.bgWidget

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.palette.error
                        font.family: Theme.palette.font
                        font.pixelSize: 14
                        font.bold: true
                    }

                    MouseArea {
                        id: closeMouse
                        anchors.fill: parent
                        enabled: !viewer.isWorking
                        onClicked: viewer.closeViewer()
                        hoverEnabled: true
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Text {
                    text: "Annotation color"
                    color: Theme.palette.tertiary
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                }

                Repeater {
                    model: ["#ff3b30", "#34c759", "#0a84ff", "#ffd60a", "#ffffff", "#000000"]
                    delegate: Rectangle {
                        id: colorPreset
                        required property var modelData
                        Layout.preferredWidth: 20
                        Layout.preferredHeight: 20
                        radius: 10
                        color: modelData
                        border.width: viewer.annotationColor == modelData ? 2 : 1
                        border.color: viewer.annotationColor == modelData ? Theme.palette.borderActive : Theme.palette.borderInactive

                        MouseArea {
                            anchors.fill: parent
                            onClicked: viewer.annotationColor = colorPreset.modelData
                        }
                    }
                }

                Item {
                    Layout.fillWidth: true
                }

                Text {
                    text: "Pen"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                }

                Slider {
                    id: penSlider
                    from: 0
                    to: 16
                    value: viewer.penSize
                    onMoved: viewer.penSize = Math.round(value)
                    Layout.preferredWidth: 120

                    background: Rectangle {
                        x: penSlider.leftPadding
                        y: penSlider.topPadding + penSlider.availableHeight / 2 - height / 2
                        width: penSlider.availableWidth
                        height: 4
                        radius: 2
                        color: Theme.palette.bgHover

                        Rectangle {
                            width: penSlider.visualPosition * parent.width
                            height: parent.height
                            radius: 2
                            color: Theme.palette.primary
                        }
                    }

                    handle: Rectangle {
                        x: penSlider.leftPadding + penSlider.visualPosition * (penSlider.availableWidth - width)
                        y: penSlider.topPadding + penSlider.availableHeight / 2 - height / 2
                        implicitWidth: 12
                        implicitHeight: 12
                        radius: 6
                        color: Theme.palette.primary
                    }
                }

                Binding {
                    target: penSlider
                    property: "value"
                    value: viewer.penSize
                    when: !penSlider.pressed
                }

                Text {
                    text: String(viewer.penSize)
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    Layout.preferredWidth: 18
                }

                Text {
                    text: "Zoom"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    Layout.leftMargin: 8
                }

                Slider {
                    id: zoomSlider
                    Layout.preferredWidth: 120
                    from: viewer.minImageZoom
                    to: viewer.maxImageZoom
                    stepSize: 0.1
                    value: viewer.minImageZoom
                    onMoved: viewer.setImageZoom(value, renderSurface.width / 2, renderSurface.height / 2) 
                    enabled: screenshotImage.status === Image.Ready

                    background: Rectangle {
                        x: zoomSlider.leftPadding
                        y: zoomSlider.topPadding + zoomSlider.availableHeight / 2 - height / 2
                        width: zoomSlider.availableWidth
                        height: 4
                        radius: 2
                        color: Theme.palette.bgHover

                        Rectangle {
                            width: zoomSlider.visualPosition * parent.width
                            height: parent.height
                            radius: 2
                            color: Theme.palette.primary
                        }
                    }

                    handle: Rectangle {
                        x: zoomSlider.leftPadding + zoomSlider.visualPosition * (zoomSlider.availableWidth - width)
                        y: zoomSlider.topPadding + zoomSlider.availableHeight / 2 - height / 2
                        implicitWidth: 12
                        implicitHeight: 12
                        radius: 6
                        color: Theme.palette.primary
                    }
                }

                Binding {
                    target: zoomSlider
                    property: "value"
                    value: viewer.imageZoom
                    when: !zoomSlider.pressed
                }

                Text {
                    text: Math.round(viewer.imageZoom * 100) + "%"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
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

                            onWheel: function (wheel) {
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

                            onPressed: function (mouse) {
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
                                    points: [
                                        {
                                            x: imagePos.x,
                                            y: imagePos.y
                                        }
                                    ]
                                };
                                strokes.push(stroke);
                                viewer.annotationStrokes = strokes;
                                paintCanvas.currentStrokeIndex = strokes.length - 1;
                                paintCanvas.requestPaint();
                            }

                            onPositionChanged: function (mouse) {
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

                                stroke.points.push({
                                    x: imagePos.x,
                                    y: imagePos.y
                                });
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
                color: viewer.statusError ? Theme.palette.error : Theme.palette.success

                Text {
                    anchors.centerIn: parent
                    text: viewer.statusMessage
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    font.bold: true
                }
            }
        }
    }

    // Off-screen compositor used for full-resolution annotation exports.
    Item {
        id: annotationExportSurface
        visible: viewer.annotatedExportPending
        opacity: 0
        x: -20000
        y: -20000
        width: Math.max(1, Math.round(viewer.annotatedExportWidth))
        height: Math.max(1, Math.round(viewer.annotatedExportHeight))

        Image {
            id: annotationExportBase
            anchors.fill: parent
            source: viewer.toFileUrl(viewer.sourcePath)
            fillMode: Image.Stretch
            smooth: false
            cache: false
        }

        Canvas {
            id: annotationExportCanvas
            anchors.fill: parent

            onPaint: {
                const ctx = getContext("2d");
                ctx.clearRect(0, 0, annotationExportCanvas.width, annotationExportCanvas.height);

                const strokes = viewer.annotationStrokes || [];
                const sourceW = Math.max(1, Number(imageDrawSurface.width || 1));
                const sourceH = Math.max(1, Number(imageDrawSurface.height || 1));
                const sx = annotationExportCanvas.width / sourceW;
                const sy = annotationExportCanvas.height / sourceH;
                const strokeScale = (sx + sy) / 2;

                for (let i = 0; i < strokes.length; i++) {
                    const stroke = strokes[i];
                    const points = stroke.points || [];
                    if (points.length < 2)
                        continue;

                    ctx.lineJoin = "round";
                    ctx.lineCap = "round";
                    ctx.strokeStyle = stroke.color;
                    ctx.lineWidth = Math.max(1, stroke.size * strokeScale);
                    ctx.beginPath();
                    ctx.moveTo(points[0].x * sx, points[0].y * sy);
                    for (let p = 1; p < points.length; p++)
                        ctx.lineTo(points[p].x * sx, points[p].y * sy);
                    ctx.stroke();
                }
            }
        }
    }
}
