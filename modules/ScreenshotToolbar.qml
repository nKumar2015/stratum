import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../theme"
import "../globals"

PanelWindow {
    id: toolbar

    property bool visibleState: false
    property bool isCapturing: false
    property string pendingCommand: ""
    property string pendingGeometry: ""
    property string pendingMode: ""
    property string lastCapturePath: ""
    property string statusMessage: ""
    property bool statusError: false
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

    function showStatus(message, isError) {
        statusMessage = String(message || "");
        statusError = !!isError;
        if (!statusMessage)
            return;
        clearStatusTimer.restart();
    }

    function openToolbar() {
        visibleState = true;
        statusMessage = "";
        statusError = false;
        resetSelectionState();
    }

    function closeToolbar() {
        visibleState = false;
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
        statusMessage = "";
        statusError = false;
        pendingCommand = chosenCommand;
        pendingMode = chosenMode;
        pendingGeometry = chosenGeometry;
        visibleState = false;
        delayedCaptureTimer.restart();
    }

    function captureFromClick() {
        if (dragActive)
            return;

        if (hoverWindowGeometry.length > 0) {
            runCapture("capture-geometry", "window", hoverWindowGeometry);
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
        id: clearStatusTimer
        interval: 2400
        repeat: false
        onTriggered: {
            toolbar.statusMessage = "";
            toolbar.statusError = false;
        }
    }

    Timer {
        id: delayedCaptureTimer
        interval: 24
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
        running: toolbar.visibleState && !toolbar.pointerDown && !toolbar.dragActive
        repeat: true
        onTriggered: {
            if (!toolbar.visibleState || toolbar.pointerDown || toolbar.dragActive || windowAtProc.running)
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
                const result = this.text.trim();

                if (!result) {
                    toolbar.visibleState = true;
                    toolbar.showStatus("Capture failed: empty response", true);
                    return;
                }

                if (result.startsWith("__ERROR__|")) {
                    const message = result.substring("__ERROR__|".length);
                    toolbar.visibleState = true;
                    toolbar.showStatus(message || "Capture failed", true);
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
                    toolbar.showStatus("Capture failed: invalid response", true);
                    return;
                }

                const imagePath = parts[1] || "";
                const captureMode = parts[2] || toolbar.selectedMode;
                if (!imagePath) {
                    toolbar.visibleState = true;
                    toolbar.showStatus("Capture failed: no file path", true);
                    return;
                }

                toolbar.lastCapturePath = imagePath;
                toolbar.statusMessage = "";
                toolbar.statusError = false;
                toolbar.visibleState = false;
                GlobalState.addNotification({
                    appName: "Screenshot",
                    summary: "Captured " + toolbar.modeLabel(captureMode),
                    body: imagePath,
                    urgency: 1,
                    category: "screenshot"
                });
                toolbar.dispatchViewerOpen(imagePath, captureMode);
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

        function openToolbar(): void {
            toolbar.openToolbar();
        }

        function toggle(): void {
            if (toolbar.visibleState)
                toolbar.closeToolbar();
            else
                toolbar.openToolbar();
        }

        function close(): void {
            toolbar.closeToolbar();
        }

        function capture(mode): void {
            const chosenMode = String(mode || "window");
            if (chosenMode === "region") {
                if (!toolbar.visibleState)
                    toolbar.openToolbar();
                return;
            }
            if (chosenMode === "fullscreen") {
                toolbar.runCapture("capture-fullscreen", "fullscreen", "");
                return;
            }
            if (!toolbar.visibleState)
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
        enabled: toolbar.visibleState
        hoverEnabled: true

        onPressed: function(mouse) {
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

    Rectangle {
        width: 320
        height: 32
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: 20
        radius: 8
        color: "#aa000000"
        border.width: 1
        border.color: "#66ffffff"
        visible: toolbar.visibleState

        Text {
            anchors.centerIn: parent
            text: toolbar.pointerDown ? "Release to capture" : "Click window, drag region, or click empty for fullscreen"
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: 11
            font.bold: true
        }
    }

    Rectangle {
        visible: toolbar.visibleState && !toolbar.dragActive && toolbar.hoverWindowGeometry.length > 0
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
        visible: toolbar.visibleState && toolbar.dragActive
        x: toolbar.dragX
        y: toolbar.dragY
        width: toolbar.dragW
        height: toolbar.dragH
        color: "#3a7aa2f7"
        border.width: 2
        border.color: "#cfe0ffff"
        radius: 3
    }

    Rectangle {
        visible: toolbar.visibleState && toolbar.statusMessage.length > 0
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: 104
        radius: 8
        color: toolbar.statusError ? "#662222" : "#1c2c1c"
        border.width: 1
        border.color: toolbar.statusError ? "#aa4444" : "#3f8f3f"
        width: statusText.implicitWidth + 20
        height: 30

        Text {
            id: statusText
            anchors.centerIn: parent
            text: toolbar.statusMessage
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: 11
            font.bold: true
        }
    }
}
