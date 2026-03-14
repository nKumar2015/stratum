pragma Singleton
import QtQuick

QtObject {
    property bool showWifiSettings: false
    property bool showWifiHoverMenu: false
    property real wifiIconY: 0
    property bool wifiHoverIntent: false
    property bool showBluetoothSettings: false
    property bool showBluetoothHoverMenu: false
    property real bluetoothIconY: 0
    property bool bluetoothHoverIntent: false
    property bool bluetoothPowered: false
    property bool bluetoothConnected: false
    property bool bluetoothScanning: false
}
