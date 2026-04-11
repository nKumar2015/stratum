pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property real thickness: 1
    property color dividerColor: Theme.palette.secondary

    Layout.fillWidth: true
    implicitHeight: thickness
    color: dividerColor
}