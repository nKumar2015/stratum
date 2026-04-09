import QtQuick

QtObject {
    // Base Backgrounds
    property color bgMain: "#1f1d2e"       // Surface - Main bar background
    property color bgWidget: "#26233a"     // Overlay - Elevated elements (menus, sidebars)
    property color bgHover: "#403d52"      // Highlight Med - Button and module hover states
    property color bgDark: "#191724"       // Base - Deepest shade for inner gaps

    // Text & Foreground
    property color textMain: "#e0def4"     // Text - Primary text and active icons
    property color textMuted: "#908caa"    // Subtle - Inactive workspaces, dim text

    // Accents
    property color primary: "#c4a7e7"      // Iris (Purple) - Active states, primary highlights
    property color secondary: "#ebbcba"    // Rose (Pink) - Sliders, secondary highlights
    property color tertiary: "#31748f"     // Pine (Teal) - Decorative elements (thick shapes)

    // Status Colors
    property color success: "#9ccfd8"      // Foam (Aqua) - Good battery, WiFi connected
    property color warning: "#f6c177"      // Gold - Low battery, pending updates
    property color error: "#eb6f92"        // Love (Red) - Critical battery, muted mic

    // Borders
    property color borderActive: "#c4a7e7" // Matches Iris (Primary)
    property color borderInactive: "#191724" // Matches Base

    // Font
    property string font: "JetBrainsMono NFM"
}
