import QtQuick
import QtQuick.Controls

import "../globals"

ToolTip {
    id: styledTip

    delay: 250
    timeout: 1200

    contentItem: Text {
        text: styledTip.text
        color: Theme.palette.textMain
        font.family: Theme.palette.font
        font.pixelSize: 10
    }

    background: Rectangle {
        color: Theme.palette.bgWidget
        radius: 5
    }
}
