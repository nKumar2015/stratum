import QtQuick

import "../globals"

QtObject {
    id: root

    required property QtObject viewer
    required property Item annotationExportSurface
    required property Canvas annotationExportCanvas

    function handleSaveAsPortalError(message) {
        const detail = String(message || "unknown error");
        viewer.logSaveAs("branch=error message=" + detail);
        viewer.showStatus(viewer.saveAsError(detail), true);
    }

    function handleSaveAsPortalSuccess(payload) {
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

    function handlePortalSaveAsResult(rawResult) {
        const result = String(rawResult || "").trim();
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
            handleSaveAsPortalError(payload.error);
            return;
        }

        handleSaveAsPortalSuccess(payload);
    }

    function notifyPostActionFailure(message) {
        GlobalState.addNotification({
            appName: "Screenshot",
            summary: "Viewer action failed",
            body: message || "Unknown error",
            urgency: 2,
            category: "screenshot"
        });
    }

    function handlePostActionFailure(message, shouldNotify) {
        const detail = String(message || "Action failed");
        viewer.showStatus(detail || "Action failed", true);
        if (!!shouldNotify)
            notifyPostActionFailure(detail);
    }

    function handlePostActionResult(rawResult) {
        const result = String(rawResult || "").trim();
        if (!result) {
            handlePostActionFailure("Action failed: empty response", false);
            return;
        }

        const payload = viewer.parseCliJson(result);
        if (!payload) {
            handlePostActionFailure("Action failed: invalid response", false);
            return;
        }

        if (payload.ok !== true) {
            const message = String(payload.error || "Action failed");
            handlePostActionFailure(message || "Action failed", true);
            return;
        }

        const action = String(payload.action || "");
        if (!action.length) {
            handlePostActionFailure("Action failed", false);
            return;
        }

        viewer.showStatus(viewer.statusForPostAction(action), false);
    }

    function clearAnnotatedExportState() {
        viewer.annotatedExportPending = false;
        viewer.pendingAnnotatedAction = "";
        viewer.pendingAnnotatedTargetPath = "";
    }

    function failAnnotatedExport(message, cleanupTempFile) {
        clearAnnotatedExportState();
        viewer.isWorking = false;
        if (!!cleanupTempFile)
            viewer.cleanupPostActionTempPath();
        viewer.showStatus(message, true);
    }

    function grabAnnotatedExportImage() {
        annotationExportCanvas.requestPaint();
        annotationExportSurface.grabToImage(function (result) {
            const exportPath = viewer.toLocalPath(viewer.postActionTempPath);
            const action = viewer.pendingAnnotatedAction;
            const target = viewer.pendingAnnotatedTargetPath;

            clearAnnotatedExportState();

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
