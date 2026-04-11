pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland
import Quickshell.Io

import "../globals"
import "../components"

PanelWindow {
    id: switcher
    property int selectedIndex: -1
    readonly property string normalizedSearch: String(searchField.text || "").toLowerCase().trim()
    readonly property var filteredThemes: {
        const query = switcher.normalizedSearch;
        const list = Theme.availableThemes || [];
        const out = [];

        for (let i = 0; i < list.length; i++) {
            const name = String(list[i]);
            if (!query || name.toLowerCase().indexOf(query) !== -1)
                out.push(name);
        }

        return out;
    }

    anchors {
        top: true
        bottom: true
        left: true
        right: true
    }

    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: visible ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    color: "#90000000"
    visible: false

    function applyTheme(themeName: string): void {
        if (Theme.switchTheme(themeName))
            switcher.visible = false;
    }

    function indexForCurrentTheme(): int {
        for (let i = 0; i < switcher.filteredThemes.length; i++) {
            if (String(switcher.filteredThemes[i]) === String(Theme.currentTheme))
                return i;
        }
        return switcher.filteredThemes.length > 0 ? 0 : -1;
    }

    function clampSelection(): void {
        const count = switcher.filteredThemes.length;
        if (count <= 0) {
            switcher.selectedIndex = -1;
            return;
        }

        if (switcher.selectedIndex < 0 || switcher.selectedIndex >= count)
            switcher.selectedIndex = 0;
    }

    function moveSelection(delta: int): void {
        const count = switcher.filteredThemes.length;
        if (count <= 0)
            return;

        if (switcher.selectedIndex < 0 || switcher.selectedIndex >= count)
            switcher.selectedIndex = 0;
        else
            switcher.selectedIndex = (switcher.selectedIndex + delta + count) % count;
    }

    function applySelectedTheme(): void {
        if (switcher.selectedIndex < 0 || switcher.selectedIndex >= switcher.filteredThemes.length)
            return;

        switcher.applyTheme(String(switcher.filteredThemes[switcher.selectedIndex]));
    }

    onFilteredThemesChanged: switcher.clampSelection()

    IpcHandler {
        target: "theme"

        function open(): void {
            switcher.visible = true;
            searchField.text = "";
            switcher.selectedIndex = switcher.indexForCurrentTheme();
            searchField.forceActiveFocus();
        }

        function close(): void {
            switcher.visible = false;
        }

        function toggle(): void {
            switcher.visible = !switcher.visible;
        }

        function set(themeName: string): void {
            switcher.applyTheme(themeName);
        }
    }

    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (switcher.visible)
                switcher.visible = false;
        }
    }

    Shortcut {
        sequence: "Down"
        onActivated: {
            if (switcher.visible)
                switcher.moveSelection(1);
        }
    }

    Shortcut {
        sequence: "Up"
        onActivated: {
            if (switcher.visible)
                switcher.moveSelection(-1);
        }
    }

    Shortcut {
        sequence: "Return"
        onActivated: {
            if (switcher.visible)
                switcher.applySelectedTheme();
        }
    }

    MouseArea {
        anchors.fill: parent
        onClicked: switcher.visible = false
    }

    Rectangle {
        anchors.centerIn: parent
        width: 360
        implicitHeight: contentColumn.implicitHeight + 32
        color: Theme.palette.bgMain
        border.width: 1
        border.color: Theme.palette.borderActive
        radius: 12

        MouseArea {
            anchors.fill: parent
        }

        ColumnLayout {
            id: contentColumn
            anchors.fill: parent
            anchors.margins: 16
            spacing: 12

            Text {
                Layout.fillWidth: true
                text: "Theme Switcher"
                color: Theme.palette.textMain
                font.pixelSize: 20
                font.bold: true
                font.family: Theme.palette.font
            }

            Text {
                Layout.fillWidth: true
                text: "Current: " + Theme.currentTheme
                color: Theme.palette.textMuted
                font.pixelSize: 13
                font.family: Theme.palette.font
            }

            TextField {
                id: searchField
                Layout.fillWidth: true
                Layout.preferredHeight: 34
                placeholderText: "Search themes"
                selectByMouse: true
                color: Theme.palette.textMain
                placeholderTextColor: Theme.palette.textMuted
                font.pixelSize: 13
                font.family: Theme.palette.font

                background: Rectangle {
                    radius: 8
                    color: Theme.palette.bgWidget
                    border.width: 1
                    border.color: Theme.palette.borderInactive
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 36
                radius: 8
                visible: switcher.filteredThemes.length === 0
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "No matching themes"
                    color: Theme.palette.textMuted
                    font.pixelSize: 12
                    font.family: Theme.palette.font
                }
            }

            Repeater {
                model: switcher.filteredThemes

                delegate: HoverListRow {
                    required property var modelData
                    required property int index

                    readonly property bool isCurrentTheme: Theme.currentTheme === String(modelData)
                    readonly property bool isKeyboardSelected: switcher.selectedIndex === index

                    Layout.fillWidth: true
                    rowHeight: 42
                    labelText: String(modelData)
                    isActive: isCurrentTheme || isKeyboardSelected
                    onHoveredChanged: hovered => {
                        if (hovered)
                            switcher.selectedIndex = index;
                    }
                    onClicked: mouse => switcher.applyTheme(String(modelData))
                }
            }
        }
    }
}
