import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: toolbar

    property bool visibleState: false
    property bool isCapturing: false
    property bool suppressOverlayVisuals: false
    property bool freezeReady: false
    property string freezeFramePath: ""
    property string pendingCommand: ""
    property string pendingGeometry: ""
    property string pendingMode: ""
    property string lastCapturePath: ""
    property real hoverWindowX: 0
    property real hoverWindowY: 0
    property real hoverWindowW: 0
    property real hoverWindowH: 0
    property string hoverWindowGeometry: ""
    property bool pointerDown: false
    property bool dragActive: false
    property real pressX: 0
    property real pressY: 0
    property real dragX: 0
    property real dragY: 0
    property real dragW: 0
    property real dragH: 0
    readonly property real dragThreshold: 8

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    color: "transparent"
    visible: visibleState
    exclusiveZone: -1

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

    function modeLabel(mode) {
        if (mode === "window")
            return "Window";
        if (mode === "region")
            return "Region";
        if (mode === "fullscreen")
            return "Fullscreen";
        return mode;
    }

    function toFileUrl(path) {
        const value = String(path || "");
        if (!value)
            return "";
        if (value.startsWith("file://"))
            return value;
        return "file://" + encodeURI(value);
    }

    function resetSelectionState() {
        hoverWindowX = 0;
        hoverWindowY = 0;
        hoverWindowW = 0;
        hoverWindowH = 0;
        hoverWindowGeometry = "";
        pointerDown = false;
        dragActive = false;
        dragX = 0;
        dragY = 0;
        dragW = 0;
        dragH = 0;
    }

    function parseGeometry(geometry) {
        const text = String(geometry || "").trim();
        const match = /^(-?\d+),(-?\d+)\s+(\d+)x(\d+)$/.exec(text);
        if (!match)
            return null;
        return {
            x: Number(match[1]),
            y: Number(match[2]),
            w: Number(match[3]),
            h: Number(match[4])
        };
    }

    function updateHoverFromGeometry(geometry) {
        const parsed = parseGeometry(geometry);
        if (!parsed) {
            hoverWindowGeometry = "";
            hoverWindowX = 0;
            hoverWindowY = 0;
            hoverWindowW = 0;
            hoverWindowH = 0;
            return;
        }

        hoverWindowGeometry = String(geometry || "");
        hoverWindowX = parsed.x;
        hoverWindowY = parsed.y;
        hoverWindowW = parsed.w;
        hoverWindowH = parsed.h;
    }

    function updateDragRect(mouseX, mouseY) {
        const dx = mouseX - pressX;
        const dy = mouseY - pressY;
        const absDx = Math.abs(dx);
        const absDy = Math.abs(dy);
        if (!dragActive && (absDx >= dragThreshold || absDy >= dragThreshold))
            dragActive = true;

        if (!dragActive)
            return;

        dragX = Math.min(pressX, mouseX);
        dragY = Math.min(pressY, mouseY);
        dragW = Math.max(1, absDx);
        dragH = Math.max(1, absDy);
    }

    function expandGeometry(geometry, marginPx) {
        const parsed = parseGeometry(geometry);
        if (!parsed)
            return geometry;

        const margin = Math.max(0, Number(marginPx || 0));
        const x = Math.round(parsed.x - margin);
        const y = Math.round(parsed.y - margin);
        const w = Math.round(parsed.w + margin * 2);
        const h = Math.round(parsed.h + margin * 2);
        return x + "," + y + " " + Math.max(1, w) + "x" + Math.max(1, h);
    }

    function openToolbar() {
        visibleState = true;
        suppressOverlayVisuals = false;
        freezeReady = false;
        freezeFramePath = "";
        resetSelectionState();
        if (!freezeProc.running)
            freezeStartTimer.restart();
    }

    function closeToolbar() {
        visibleState = false;
        suppressOverlayVisuals = false;
        freezeReady = false;
        if (!isCapturing) {
            pendingMode = "";
            pendingGeometry = "";
            pendingCommand = "";
        }
        resetSelectionState();
    }

    function runCapture(commandName, mode, geometry) {
        if (isCapturing)
            return;

        const chosenCommand = String(commandName || "capture-fullscreen");
        const chosenMode = String(mode || "fullscreen");
        const chosenGeometry = String(geometry || "");
        pendingCommand = chosenCommand;
        pendingMode = chosenMode;
        pendingGeometry = chosenGeometry;
        suppressOverlayVisuals = true;
        delayedCaptureTimer.restart();
    }

    function captureFromClick() {
        if (dragActive)
            return;

        if (hoverWindowGeometry.length > 0) {
            runCapture("capture-geometry", "window", expandGeometry(hoverWindowGeometry, 5));
            return;
        }

        runCapture("capture-fullscreen", "fullscreen", "");
    }

    function captureFromDrag() {
        if (!dragActive)
            return;

        const geometry = Math.round(dragX) + "," + Math.round(dragY) + " " + Math.round(dragW) + "x" + Math.round(dragH);
        runCapture("capture-geometry", "region", geometry);
    }

    function dispatchViewerOpen(path, mode) {
        GlobalState.screenshotViewerOpenRequested(path, mode);
    }

    Timer {
        id: freezeStartTimer
        interval: 80
        repeat: false
        onTriggered: {
            if (!toolbar.visibleState || freezeProc.running)
                return;
            freezeProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_menu.sh", "freeze-frame"];
            freezeProc.running = true;
        }
    }

    Timer {
        id: delayedCaptureTimer
        interval: 120
        repeat: false
        onTriggered: {
            const nextCommand = toolbar.pendingCommand;
            const nextMode = toolbar.pendingMode;
            if (!nextCommand)
                return;

            const nextGeometry = toolbar.pendingGeometry;
            toolbar.pendingCommand = "";
            toolbar.pendingMode = "";
            toolbar.pendingGeometry = "";
            toolbar.isCapturing = true;
            toolbar.freezeReady = false;
            toolbar.resetSelectionState();

            if (nextCommand === "capture-geometry")
                captureProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_menu.sh", "capture-geometry", nextGeometry, nextMode];
            else
                captureProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_menu.sh", "capture-fullscreen", nextMode];

            captureProc.running = true;
        }
    }

    Timer {
        id: hoverPollTimer
        interval: 45
        running: toolbar.visibleState && !toolbar.suppressOverlayVisuals && toolbar.freezeReady && !toolbar.pointerDown && !toolbar.dragActive
        repeat: true
        onTriggered: {
            if (!toolbar.visibleState || toolbar.suppressOverlayVisuals || !toolbar.freezeReady || toolbar.pointerDown || toolbar.dragActive || windowAtProc.running)
                return;

            windowAtProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_menu.sh", "window-at", String(Math.round(pointerArea.mouseX)), String(Math.round(pointerArea.mouseY))];
            windowAtProc.running = true;
        }
    }

    Process {
        id: captureProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                toolbar.isCapturing = false;
                toolbar.suppressOverlayVisuals = false;
                const result = this.text.trim();

                if (!result) {
                    toolbar.visibleState = true;
                    toolbar.freezeReady = true;
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Capture failed",
                        body: "Empty response from capture process",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                if (result.startsWith("__ERROR__|")) {
                    const message = result.substring("__ERROR__|".length);
                    toolbar.visibleState = true;
                    toolbar.freezeReady = true;
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Capture failed",
                        body: message || "Unknown error",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                const parts = result.split("|");
                if (parts.length < 4 || parts[0] !== "ok") {
                    toolbar.visibleState = true;
                    toolbar.freezeReady = true;
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Capture failed",
                        body: "Invalid response from capture process",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                const imagePath = parts[1] || "";
                const captureMode = parts[2] || "fullscreen";
                if (!imagePath) {
                    toolbar.visibleState = true;
                    toolbar.freezeReady = true;
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Capture failed",
                        body: "No file path returned",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                toolbar.lastCapturePath = imagePath;
                toolbar.visibleState = false;
                toolbar.dispatchViewerOpen(imagePath, captureMode);
            }
        }
    }

    Process {
        id: freezeProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (!toolbar.visibleState)
                    return;

                if (!result) {
                    toolbar.freezeReady = false;
                    toolbar.closeToolbar();
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Start failed",
                        body: "Could not freeze screen",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                if (result.startsWith("__ERROR__|")) {
                    const message = result.substring("__ERROR__|".length);
                    toolbar.freezeReady = false;
                    toolbar.closeToolbar();
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Start failed",
                        body: message || "Could not freeze screen",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                const parts = result.split("|");
                if (parts.length < 2 || parts[0] !== "ok") {
                    toolbar.freezeReady = false;
                    toolbar.closeToolbar();
                    GlobalState.addNotification({
                        appName: "Screenshot",
                        summary: "Start failed",
                        body: "Invalid freeze response",
                        urgency: 2,
                        category: "screenshot"
                    });
                    return;
                }

                toolbar.freezeFramePath = parts[1] || "";
                toolbar.freezeReady = toolbar.freezeFramePath.length > 0;
                toolbar.suppressOverlayVisuals = false;
            }
        }
    }

    Process {
        id: windowAtProc
        running: false
        stdout: StdioCollector {
            onStreamFinished: {
                const result = this.text.trim();
                if (!result)
                    return;

                if (result.startsWith("ok|")) {
                    const parts = result.split("|");
                    toolbar.updateHoverFromGeometry(parts[1] || "");
                    return;
                }

                if (result === "none")
                    toolbar.updateHoverFromGeometry("");
            }
        }
    }

    IpcHandler {
        target: "screenshot"

        function start(): void {
            toolbar.openToolbar();
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (toolbar.visibleState)
                toolbar.closeToolbar();
        }
    }

    MouseArea {
        id: pointerArea
        anchors.fill: parent
        enabled: toolbar.visibleState && !toolbar.suppressOverlayVisuals && !toolbar.isCapturing
        hoverEnabled: true
        cursorShape: Qt.BlankCursor

        onPressed: function(mouse) {
            if (!toolbar.freezeReady)
                return;
            toolbar.pointerDown = true;
            toolbar.dragActive = false;
            toolbar.pressX = mouse.x;
            toolbar.pressY = mouse.y;
            toolbar.dragX = mouse.x;
            toolbar.dragY = mouse.y;
            toolbar.dragW = 1;
            toolbar.dragH = 1;
        }

        onPositionChanged: function(mouse) {
            if (!toolbar.freezeReady)
                return;
            if (toolbar.pointerDown)
                toolbar.updateDragRect(mouse.x, mouse.y);
        }

        onReleased: {
            const hadDrag = toolbar.dragActive;
            toolbar.pointerDown = false;
            if (hadDrag)
                toolbar.captureFromDrag();
            else
                toolbar.captureFromClick();
            toolbar.dragActive = false;
        }

        onCanceled: {
            toolbar.pointerDown = false;
            toolbar.dragActive = false;
        }
    }

    Image {
        anchors.fill: parent
        visible: toolbar.visibleState && !toolbar.suppressOverlayVisuals && toolbar.freezeReady && toolbar.freezeFramePath.length > 0
        source: toolbar.toFileUrl(toolbar.freezeFramePath)
        fillMode: Image.Stretch
        sourceSize.width: Math.max(1, Math.round(width * Screen.devicePixelRatio))
        sourceSize.height: Math.max(1, Math.round(height * Screen.devicePixelRatio))
        smooth: false
        cache: false
    }

    Rectangle {
        visible: toolbar.visibleState && !toolbar.suppressOverlayVisuals && !toolbar.dragActive && toolbar.hoverWindowGeometry.length > 0
        x: toolbar.hoverWindowX
        y: toolbar.hoverWindowY
        width: toolbar.hoverWindowW
        height: toolbar.hoverWindowH
        color: "#3a7aa2f7"
        border.width: 2
        border.color: "#bcd6ffff"
        radius: 6
    }

    Rectangle {
        visible: toolbar.visibleState && !toolbar.suppressOverlayVisuals && toolbar.dragActive
        x: toolbar.dragX
        y: toolbar.dragY
        width: toolbar.dragW
        height: toolbar.dragH
        color: "#3a7aa2f7"
        border.width: 2
        border.color: "#cfe0ffff"
        radius: 3
    }
}
