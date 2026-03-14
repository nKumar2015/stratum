pragma Singleton
import QtQuick

QtObject {
    property bool showWifiSettings: false
    property bool showWifiHoverMenu: false
    property real wifiIconY: 0
    property bool wifiHoverIntent: false
    property bool showAudioHoverMenu: false
    property real audioIconY: 0
    property bool audioHoverIntent: false
    property int audioVolumePercent: 0
    property bool audioMuted: true
    property bool audioUserAdjusting: false
    property bool showBluetoothSettings: false
    property bool showBluetoothHoverMenu: false
    property real bluetoothIconY: 0
    property bool bluetoothHoverIntent: false
    property bool bluetoothPowered: false
    property bool bluetoothConnected: false
    property bool bluetoothScanning: false
    property bool showBatteryHoverMenu: false
    property real batteryIconY: 0
    property bool batteryHoverIntent: false
}
