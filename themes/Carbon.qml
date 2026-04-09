import QtQuick

QtObject {
    // Base Backgrounds
    property color bgMain: "#161616"       // Main bar background
    property color bgWidget: "#262626"     // Elevated elements (menus, sidebars)
    property color bgHover: "#393939"      // Button and module hover states
    property color bgDark: "#15161e"       // Deepest shade for inner gaps

    // Text & Foreground
    property color textMain: "#f4f4f4"     // Primary text and active icons
    property color textMuted: "#8d8d8d"   // Inactive workspaces, dim text

    // Accents
    property color primary: "#4589ff"      // Blue - Active states, primary highlights
    property color secondary: "#a56eff"    // Purple - Sliders, secondary highlights
    property color tertiary: "#33b1ff"     // Cyan - Decorative elements

    // Status Colors
    property color success: "#42be64"      // Green - Good battery, WiFi connected
    property color warning: "#f1c21b"      // Yellow - Low battery, pending updates
    property color error: "#fa4d56"        // Red - Critical battery, muted mic

   // Borders
    property color borderActive: "#4589ff"
    property color borderInactive: "#393939"

    // Font
    property string font: "JetBrainsMono NFM"
}
