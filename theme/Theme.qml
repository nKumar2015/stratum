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
    property color green: "#98c379"
    property color yellow: "#e5c07b"
    property color red: "#e06c75"
    property color purple: "#c678dd"
    property color blue: "#89b4fa"
    property color black: "#11111b"
    property color white: "#cdd6f4"
    property color grey: "#313244"

    // Notification colors
    property color notificationBg: "#171923"
    property color notificationBorder: "#2f3448"
    property color notificationLow: "#7f849c"
    property color notificationNormal: "#89b4fa"
    property color notificationCritical: "#e06c75"
    property color notificationMuted: "#6c7086"
    // Font
    property var font: "JetBrainsMono NFM"
}
