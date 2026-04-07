import QtQuick

QtObject {
    id: root

    required property QtObject viewer
    required property Item renderSurface
    required property Image screenshotImage
    required property Item imageDrawSurface

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

        const oldZoom = viewer.imageZoom;
        const clampedZoom = clamp(Number(nextZoom || 1), viewer.minImageZoom, viewer.maxImageZoom);
        const fx = (typeof focusX === "number") ? focusX : (renderSurface.width / 2);
        const fy = (typeof focusY === "number") ? focusY : (renderSurface.height / 2);

        const oldPosX = (renderSurface.width - (baseW * oldZoom)) / 2 + viewer.imagePanX;
        const oldPosY = (renderSurface.height - (baseH * oldZoom)) / 2 + viewer.imagePanY;
        const contentX = (fx - oldPosX) / Math.max(0.0001, oldZoom);
        const contentY = (fy - oldPosY) / Math.max(0.0001, oldZoom);

        const newPosX = fx - contentX * clampedZoom;
        const newPosY = fy - contentY * clampedZoom;
        const centeredPosX = (renderSurface.width - (baseW * clampedZoom)) / 2;
        const centeredPosY = (renderSurface.height - (baseH * clampedZoom)) / 2;
        const unclampedPanX = newPosX - centeredPosX;
        const unclampedPanY = newPosY - centeredPosY;
        const clampedPan = clampPanForZoom(clampedZoom, unclampedPanX, unclampedPanY);

        viewer.imageZoom = clampedZoom;
        viewer.imagePanX = clampedPan.x;
        viewer.imagePanY = clampedPan.y;
    }

    function startPan(surfaceX, surfaceY) {
        viewer.panActive = true;
        viewer.panLastSurfaceX = surfaceX;
        viewer.panLastSurfaceY = surfaceY;
    }

    function updatePan(surfaceX, surfaceY) {
        if (!viewer.panActive)
            return;

        const dx = surfaceX - viewer.panLastSurfaceX;
        const dy = surfaceY - viewer.panLastSurfaceY;
        viewer.panLastSurfaceX = surfaceX;
        viewer.panLastSurfaceY = surfaceY;

        const clampedPan = clampPanForZoom(viewer.imageZoom, viewer.imagePanX + dx, viewer.imagePanY + dy);
        viewer.imagePanX = clampedPan.x;
        viewer.imagePanY = clampedPan.y;
    }

    function stopPan() {
        viewer.panActive = false;
    }

    function clearAnnotations() {
        viewer.annotationStrokes = [];
    }

    function undoLastAnnotation() {
        const strokes = viewer.annotationStrokes || [];
        if (!strokes.length)
            return;

        viewer.annotationStrokes = strokes.slice(0, strokes.length - 1);
    }

    function hasAnnotations() {
        return (viewer.annotationStrokes || []).length > 0;
    }

    function colorToHex(c) {
        const r = Math.round(c.r * 255).toString(16).padStart(2, "0");
        const g = Math.round(c.g * 255).toString(16).padStart(2, "0");
        const b = Math.round(c.b * 255).toString(16).padStart(2, "0");
        return "#" + r + g + b;
    }
}
