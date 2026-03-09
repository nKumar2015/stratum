import QtQuick
import QtQuick.Shapes

import "../theme"

Shape {
    id: corner
    width: 20
    height: 20

    property color color: Theme.background
    property bool flip: false // false = Top, true = Bottom

    layer.enabled: true
    layer.samples: 4

    ShapePath {
        fillColor: corner.color
        strokeColor: "transparent"

        // TOP CORNER LOGIC
        // We want to fill the area that connects the vertical sidebar (left)
        // to the horizontal screen edge (top).
        startX: 0
        startY: flip ? 0 : 20

        // 1. Move to the corner point (where sidebar top meets screen edge)
        PathLine {
            x: 0
            y: flip ? 20 : 0
        }

        // 2. Move along the screen edge (to the right)
        PathLine {
            x: 20
            y: flip ? 20 : 0
        }

        // 3. Curve back to the sidebar edge
        // The control point is the corner itself (0,0 or 0,20),
        // which "pulls" the curve to create the inverted scoop.
        PathQuad {
            x: 0
            y: flip ? 0 : 20
            controlX: 0
            controlY: flip ? 20 : 0
        }
    }
}
