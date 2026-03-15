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
    property string selectedMode: "window"
    property bool isCapturing: false
    property string pendingMode: ""
    property string lastCapturePath: ""
    property string statusMessage: ""
    property bool statusError: false

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
        selectedMode = "window";
    }

    function closeToolbar() {
        visibleState = false;
        if (!isCapturing)
            pendingMode = "";
    }

    function runCapture(mode) {
        if (isCapturing)
            return;

        const chosenMode = String(mode || selectedMode || "window");
        selectedMode = chosenMode;
        statusMessage = "";
        statusError = false;
        pendingMode = chosenMode;
        visibleState = false;
        delayedCaptureTimer.restart();
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
            const nextMode = toolbar.pendingMode;
            if (!nextMode)
                return;

            toolbar.pendingMode = "";
            toolbar.isCapturing = true;
            captureProc.command = ["sh", Quickshell.shellDir + "/scripts/screenshot_menu.sh", "capture", nextMode];
            captureProc.running = true;
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
            if (!toolbar.visibleState)
                toolbar.openToolbar();
            toolbar.runCapture(mode);
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
        anchors.fill: parent
        enabled: toolbar.visibleState
        onClicked: toolbar.closeToolbar()
    }

    Rectangle {
        width: Math.max(520, controlsLayout.implicitWidth + 24)
        height: 84
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: 26
        radius: 12
        color: Theme.background
        border.width: 1
        border.color: Theme.grey
        visible: toolbar.visibleState

        MouseArea {
            anchors.fill: parent
            onClicked: {
            }
        }

        RowLayout {
            id: controlsLayout
            anchors.fill: parent
            anchors.leftMargin: 12
            anchors.rightMargin: 12
            anchors.topMargin: 8
            anchors.bottomMargin: 8
            spacing: 8

            Repeater {
                model: [
                    { key: "window", label: "Window" },
                    { key: "region", label: "Region" },
                    { key: "fullscreen", label: "Full" }
                ]

                delegate: Rectangle {
                    required property var modelData

                    readonly property bool active: toolbar.selectedMode === modelData.key
                    Layout.preferredHeight: 38
                    Layout.preferredWidth: modelData.key === "fullscreen" ? 64 : 88
                    radius: 8
                    color: active ? Theme.activeWs : Theme.black
                    border.width: 1
                    border.color: active ? Theme.activeWs : Theme.grey

                    Text {
                        anchors.centerIn: parent
                        text: modelData.label
                        color: Theme.text
                        font.family: Theme.font
                        font.pixelSize: 12
                        font.bold: true
                    }

                    MouseArea {
                        anchors.fill: parent
                        enabled: !toolbar.isCapturing
                        onClicked: toolbar.runCapture(modelData.key)
                    }
                }
            }

            Rectangle {
                Layout.preferredHeight: 38
                Layout.preferredWidth: 72
                radius: 8
                color: "#2a2a2a"
                border.width: 1
                border.color: Theme.grey

                Text {
                    anchors.centerIn: parent
                    text: "OCR"
                    color: Theme.hover
                    font.family: Theme.font
                    font.pixelSize: 12
                    font.bold: true
                }

                MouseArea {
                    anchors.fill: parent
                    enabled: false
                }
            }
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 8
            text: toolbar.isCapturing ? "Select on screen..." : "Click a mode, then release mouse to capture"
            color: Theme.hover
            font.family: Theme.font
            font.pixelSize: 10
            font.bold: true
        }
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
