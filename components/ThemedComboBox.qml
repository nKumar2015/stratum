pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls

import "../globals"

ComboBox {
    id: root

    property var items: []
    property string selectedName: ""
    property string placeholderText: "Select item"
    property int popupMaxHeight: 220
    property var labelProvider: null

    signal itemChosen(var item, int index)

    implicitHeight: 36
    model: root.items
    leftPadding: 12
    rightPadding: 28
    hoverEnabled: true

    function itemLabel(item) {
        if (typeof root.labelProvider === "function") {
            const custom = root.labelProvider(item);
            if (custom !== undefined && custom !== null && String(custom).length > 0)
                return String(custom);
        }

        const description = item && item.description ? String(item.description).trim() : "";
        if (description.length > 0)
            return description;

        return item && item.name ? String(item.name) : "";
    }

    currentIndex: {
        for (let i = 0; i < root.items.length; i++) {
            if (String(root.items[i]?.name || "") === String(root.selectedName || ""))
                return i;
        }
        return -1;
    }

    textRole: "name"
    displayText: {
        if (currentIndex < 0 || currentIndex >= root.items.length)
            return root.placeholderText;
        return root.itemLabel(root.items[currentIndex]);
    }

    delegate: ItemDelegate {
        required property int index
        required property var modelData

        width: root.width
        implicitHeight: 34
        hoverEnabled: true
        highlighted: root.highlightedIndex === index

        contentItem: Text {
            text: root.itemLabel(parent.modelData)
            color: root.currentIndex === parent.index ? Theme.palette.textMain : Theme.palette.textMuted
            font.family: Theme.palette.font
            font.pixelSize: 12
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }

        background: Rectangle {
            radius: 6
            color: parent.highlighted ? Theme.palette.bgHover : "transparent"
            border.width: root.currentIndex === parent.index ? 1 : 0
            border.color: Theme.palette.borderActive
        }
    }

    indicator: Text {
        text: "▾"
        color: Theme.palette.textMuted
        font.family: Theme.palette.font
        font.pixelSize: 12
        anchors.right: parent.right
        anchors.rightMargin: 10
        anchors.verticalCenter: parent.verticalCenter
    }

    contentItem: Text {
        text: root.displayText
        color: Theme.palette.textMain
        font.family: Theme.palette.font
        font.pixelSize: 12
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
        leftPadding: 0
        rightPadding: 0
    }

    background: Rectangle {
        radius: 8
        color: root.popup.visible || root.hovered ? Theme.palette.bgHover : Theme.palette.bgWidget
        border.width: 1
        border.color: root.popup.visible ? Theme.palette.borderActive : Theme.palette.borderInactive
    }

    popup: Popup {
        y: root.height + 6
        width: root.width
        padding: 4

        background: Rectangle {
            radius: 8
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderActive
        }

        contentItem: ListView {
            clip: true
            implicitHeight: Math.min(contentHeight, root.popupMaxHeight)
            model: root.popup.visible ? root.delegateModel : null
            currentIndex: root.highlightedIndex
            ScrollIndicator.vertical: ScrollIndicator {}
        }
    }

    onActivated: index => {
        if (index >= 0 && index < root.items.length)
            root.itemChosen(root.items[index], index);
    }
}
