import QtQuick

QtObject {
    // Base Backgrounds
    property color bgMain: "#1d2021"       // Main bar background
    property color bgWidget: "#282828"     // Elevated elements (menus, sidebars)
    property color bgHover: "#3c3836"      // Button and module hover states
    property color bgDark: "#141617"       // Deepest shade for inner gaps

    // Text & Foreground
    property color textMain: "#ebdbb2"     // Primary text and active icons
    property color textMuted: "#a89984"    // Inactive workspaces, dim text

    // Accents
    property color primary: "#83a598"      // Blue - Active states, primary highlights
    property color secondary: "#d3869b"    // Purple - Sliders, secondary highlights
    property color tertiary: "#fe8019"     // Cyan - Decorative elements

    // Status Colors
    property color success: "#b8bb26"      // Green - Good battery, WiFi connected
    property color warning: "#fabd2f"      // Yellow - Low battery, pending updates
    property color error: "#fb4934"        // Red - Critical battery, muted mic

    // Borders
    property color borderActive: "#83a598"
    property color borderInactive: "#141617"

    // Font
    property string font: "JetBrainsMono NFM"
}
