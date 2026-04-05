import QtQuick
import QtQuick.Layouts
import QtCore
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Hyprland
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

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    screen: resolvedScreen

    color: "#99000000"
    visible: visibleState && !portalSaveAsProc.running
    exclusiveZone: -1

    // Keep the viewer above normal UI during editing and Save As flow.
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: (visible && !portalSaveAsProc.running) ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

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
        const r = Math.round(c.r * 255).toString(16).padStart(2, "0");
        const g = Math.round(c.g * 255).toString(16).padStart(2, "0");
        const b = Math.round(c.b * 255).toString(16).padStart(2, "0");
        return "#" + r + g + b;
    }

    function clearAnnotations() {
        annotationStrokes = [];
        paintCanvas.requestPaint();
    }

    function undoLastAnnotation() {
        const strokes = viewer.annotationStrokes || [];
        if (!strokes.length)
            return;

        annotationStrokes = strokes.slice(0, strokes.length - 1);
        paintCanvas.currentStrokeIndex = -1;
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

        return {
            x: nextPanX,
            y: nextPanY
        };
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

    Timer {
        id: portalSaveAsGuard
        interval: 35000
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
        interval: 16
        repeat: false
        onTriggered: {
            if (!viewer.annotatedExportPending)
                return;

            if (annotationExportBase.status === Image.Error) {
                viewer.annotatedExportPending = false;
                viewer.pendingAnnotatedAction = "";
                viewer.pendingAnnotatedTargetPath = "";
                viewer.isWorking = false;
                viewer.cleanupPostActionTempPath();
                viewer.showStatus("Failed to load source image for export", true);
                return;
            }

            if (annotationExportBase.status !== Image.Ready) {
                viewer.annotatedExportRetries = viewer.annotatedExportRetries + 1;
                if (viewer.annotatedExportRetries <= 60) {
                    annotatedExportGrabTimer.restart();
                    return;
                }

                viewer.annotatedExportPending = false;
                viewer.pendingAnnotatedAction = "";
                viewer.pendingAnnotatedTargetPath = "";
                viewer.isWorking = false;
                viewer.cleanupPostActionTempPath();
                viewer.showStatus("Timed out preparing annotated export", true);
                return;
            }

            annotationExportCanvas.requestPaint();
            annotationExportSurface.grabToImage(function (result) {
                const exportPath = viewer.toLocalPath(viewer.postActionTempPath);
                const action = viewer.pendingAnnotatedAction;
                const target = viewer.pendingAnnotatedTargetPath;

                viewer.annotatedExportPending = false;
                viewer.pendingAnnotatedAction = "";
                viewer.pendingAnnotatedTargetPath = "";

                if (!exportPath) {
                    viewer.isWorking = false;
                    viewer.showStatus("Failed to prepare annotated image", true);
                    return;
                }

                const saved = result.saveToFile(exportPath);
                if (!saved) {
                    viewer.isWorking = false;
                    viewer.cleanupPostActionTempPath();
                    viewer.showStatus("Failed to render annotated image", true);
                    return;
                }

                viewer.runPostAction(action, exportPath, target);
            });
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
                const result = this.text.trim();
                if (!result) {
                    viewer.showStatus("Action failed: empty response", true);
                    return;
                }

                const payload = viewer.parseCliJson(result);
                if (!payload) {
                    viewer.showStatus("Action failed: invalid response", true);
                    return;
                }

                if (payload.ok !== true) {
                    const message = String(payload.error || "Action failed");
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

                const action = String(payload.action || "");
                if (!action.length) {
                    viewer.showStatus("Action failed", true);
                    return;
                }

                viewer.showStatus(viewer.statusForPostAction(action), false);
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

                const result = rawResult.trim();
                viewer.logSaveAs("stdout.trim=" + result);

                if (!result) {
                    viewer.logSaveAs("branch=no-response");
                    viewer.showStatus(viewer.saveAsError("no response from portal"), true);
                    return;
                }

                const payload = viewer.parseCliJson(result);
                if (!payload) {
                    viewer.logSaveAs("branch=invalid-json");
                    viewer.showStatus(viewer.saveAsError("invalid response"), true);
                    return;
                }

                if (payload.ok !== true) {
                    const message = String(payload.error || "unknown error");
                    viewer.logSaveAs("branch=error message=" + message);
                    viewer.showStatus(viewer.saveAsError(message), true);
                    return;
                }

                const status = String(payload.status || "");
                if (status === "cancel") {
                    viewer.logSaveAs("branch=cancel");
                    return;
                }

                if (status !== "ok") {
                    viewer.logSaveAs("branch=unexpected-response");
                    viewer.showStatus(viewer.saveAsError("unexpected response"), true);
                    return;
                }

                const selectedUri = String(payload.uri || "").trim();
                const selectedPath = viewer.toLocalPath(selectedUri);
                viewer.logSaveAs("selectedUri=" + selectedUri);
                viewer.logSaveAs("selectedPath=" + selectedPath);
                if (!selectedPath) {
                    viewer.logSaveAs("branch=invalid-destination");
                    viewer.showStatus(viewer.saveAsError("invalid destination"), true);
                    return;
                }

                viewer.logSaveAs("branch=dispatch-save-to");
                viewer.startPostActionWithTarget("save-to", selectedPath);
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
        enabled: viewer.visibleState && !portalSaveAsProc.running
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
        border.color: Theme.outlineVariant
        visible: viewer.visibleState && !portalSaveAsProc.running

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
                    color: Theme.on_Surface
                    font.family: Theme.font
                    font.pixelSize: 14
                    font.bold: true
                }

                Text {
                    text: "Mode: " + viewer.captureMode
                    color: Theme.on_Surface
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
                    color: Theme.surfaceContainerLowest
                    border.width: 1
                    border.color: Theme.outlineVariant

                    Text {
                        anchors.centerIn: parent
                        text: "󰃢"
                        color: Theme.on_Surface
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
                    color: Theme.surfaceContainerLowest
                    border.width: 1
                    border.color: Theme.primary

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆏"
                        color: Theme.on_Surface
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
                    color: Theme.surfaceContainerLowest
                    border.width: 1
                    border.color: Theme.primary

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰆓"
                        color: Theme.on_Surface
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
                    color: Theme.surfaceContainerLowest
                    border.width: 1
                    border.color: Theme.primary

                    Text {
                        anchors.centerIn: parent
                        text: viewer.isWorking ? "󰔛" : "󰉋"
                        color: Theme.on_Surface
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
                    color: Theme.surfaceContainerLowest
                    border.width: 1
                    border.color: Theme.outlineVariant

                    Text {
                        anchors.centerIn: parent
                        text: "󰅖"
                        color: Theme.on_Surface
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
                    color: Theme.on_Surface
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
                        border.color: viewer.annotationColor == modelData ? Theme.primary : Theme.outlineVariant

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
                    color: Theme.on_Surface
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
                    color: Theme.on_Surface
                    font.family: Theme.font
                    font.pixelSize: 11
                    Layout.preferredWidth: 18
                }

                Text {
                    text: "Zoom"
                    color: Theme.on_Surface
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
                    color: Theme.on_Surface
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
                color: viewer.statusError ? "#662222" : "#1c2c1c"
                border.width: 1
                border.color: viewer.statusError ? "#aa4444" : "#3f8f3f"

                Text {
                    anchors.centerIn: parent
                    text: viewer.statusMessage
                    color: Theme.on_Surface
                    font.family: Theme.font
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
