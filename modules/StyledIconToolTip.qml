import QtQuick
import QtQuick.Controls

import "../theme"

ToolTip {
    id: styledTip

    delay: 250
    timeout: 1200

    contentItem: Text {
        text: styledTip.text
        color: Theme.text
        font.family: Theme.font
        font.pixelSize: 10
    }

    background: Rectangle {
        color: "#111824"
        border.color: Theme.grey
        border.width: 1
        radius: 5
    }
}
