import QtQuick

QtObject {
    // Base Backgrounds
    property color bgMain: "#1a1b26"       // Main bar background
    property color bgWidget: "#1f2335"     // Elevated elements (menus, sidebars)
    property color bgHover: "#292e42"      // Button and module hover states
    property color bgDark: "#15161e"       // Deepest shade for inner gaps

    // Text & Foreground
    property color textMain: "#c0caf5"     // Primary text and active icons
    property color textMuted: "#565f89"    // Inactive workspaces, dim text

    // Accents
    property color primary: "#7aa2f7"      // Blue - Active states, primary highlights
    property color secondary: "#bb9af7"    // Purple - Sliders, secondary highlights
    property color tertiary: "#7dcfff"     // Cyan - Decorative elements

    // Status Colors
    property color success: "#9ece6a"      // Green - Good battery, WiFi connected
    property color warning: "#e0af68"      // Yellow - Low battery, pending updates
    property color error: "#f7768e"        // Red - Critical battery, muted mic

    // Borders
    property color borderActive: "#7aa2f7"
    property color borderInactive: "#15161e"

    // Font
    property string font: "JetBrainsMono NFM"
}
