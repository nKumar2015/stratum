pragma Singleton
import QtQuick
import Quickshell

Item {
    id: root

    property var palette: loader.item

    Loader {
        id: loader
        source: Quickshell.shellDir + "/themes/TokyoNight.qml"
    }

    function switchTheme(themeName) {
        loader.source = Quickshell.shellDir + "/themes/" + themeName + ".qml";
    }
}
