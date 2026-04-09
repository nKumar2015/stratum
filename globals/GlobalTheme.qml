pragma Singleton
import QtQuick
import Quickshell

Item {
    id: root

    property var palette: loader.item
    property string currentTheme: "Carbon"
    property var availableThemes: ["Carbon", "Gruvbox", "RosePine", "TokyoNight"]

    Loader {
        id: loader
        source: Quickshell.shellDir + "/themes/Carbon.qml"
    }

    function switchTheme(themeName) {
        const normalizedTheme = String(themeName || "").trim();
        if (!availableThemes.includes(normalizedTheme)) {
            console.warn("Theme.switchTheme: unknown theme", normalizedTheme);
            return false;
        }

        loader.source = Quickshell.shellDir + "/themes/" + normalizedTheme + ".qml";
        currentTheme = normalizedTheme;
        return true;
    }
}
