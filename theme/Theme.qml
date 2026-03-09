pragma Singleton
import QtQuick

QtObject {
    property color background: "#000000"
    property color text: "#FFFFFF"
    property color hover: "#45475a"

    // Workspace Colors
    property color activeWs: "#61afef"
    property color defaultWs: "#56b6c2"
    property color inactiveWs: "#5c6370"

    // Extra accents in case you want to use them for other widgets later
    property color green: "#98c379"    // One Dark Green
    property color yellow: "#e5c07b"    // One Dark Yellow
    property color red: "#e06c75"      // One Dark Red
    property color purple: "#c678dd"     // One Dark Purple
}
