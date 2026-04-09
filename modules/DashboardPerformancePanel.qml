import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property int cpuPercent: 0
    property string gpuPercentText: "N/A"
    property int gpuPercentValue: 0
    property string gpuSource: "N/A"
    property real ramUsedGiB: 0
    property real ramTotalGiB: 0
    property int ramPercent: 0
    property real storageUsedGiB: 0
    property real storageTotalGiB: 0
    property int storagePercent: 0

    function metricColor(percent) {
        if (percent >= 90)
            return Theme.palette.error;
        if (percent >= 75)
            return Theme.palette.warning;
        return Theme.palette.success;
    }

    implicitHeight: performanceColumn.implicitHeight + 20
    color: Theme.palette.bgMain
    radius: 10
    border.width: 1
    border.color: Theme.palette.borderInactive

    ColumnLayout {
        id: performanceColumn
        anchors.fill: parent
        anchors.margins: 10
        spacing: 6

        Text {
            text: "Performance"
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 14
            font.bold: true
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 70
            radius: 8
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderInactive

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 2

                RowLayout {
                    spacing: 4

                    Text {
                        text: ""
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }

                    Text {
                        text: "CPU"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        font.bold: true
                    }
                }

                Text {
                    text: String(root.cpuPercent) + "%"
                    color: root.metricColor(root.cpuPercent)
                    font.family: Theme.palette.font
                    font.pixelSize: 17
                    font.bold: true
                }

                DashboardMetricProgressBar {
                    percent: root.cpuPercent
                    fillColor: root.metricColor(root.cpuPercent)
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 70
            radius: 8
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderInactive

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 2

                RowLayout {
                    spacing: 4

                    Text {
                        text: "󰢮"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }

                    Text {
                        text: "GPU"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        font.bold: true
                    }

                    Text {
                        text: "|"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.gpuSource
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 9
                        elide: Text.ElideRight
                        font.bold: true
                    }
                }

                Text {
                    text: root.gpuPercentText === "N/A" ? "N/A" : root.gpuPercentText + "%"
                    color: root.gpuPercentText === "N/A" ? Theme.palette.textMuted : root.metricColor(root.gpuPercentValue)
                    font.family: Theme.palette.font
                    font.pixelSize: 17
                    font.bold: true
                }

                DashboardMetricProgressBar {
                    visible: root.gpuPercentText !== "N/A"
                    percent: root.gpuPercentValue
                    fillColor: metricColor(root.gpuPercentValue)
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 82
            radius: 8
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderInactive

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 2

                RowLayout {
                    spacing: 4

                    Text {
                        text: "󰍛"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }

                    Text {
                        text: "RAM"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        font.bold: true
                    }
                }

                Text {
                    text: root.ramUsedGiB.toFixed(1) + " / " + root.ramTotalGiB.toFixed(1) + " GiB"
                    color: root.metricColor(root.ramPercent)
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                }

                Text {
                    text: String(root.ramPercent) + "%"
                    color: root.metricColor(root.ramPercent)
                    font.family: Theme.palette.font
                    font.pixelSize: 13
                    font.bold: true
                }

                DashboardMetricProgressBar {
                    percent: root.ramPercent
                    fillColor: root.metricColor(root.ramPercent)
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 82
            radius: 8
            color: Theme.palette.bgWidget
            border.width: 1
            border.color: Theme.palette.borderInactive

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 2

                RowLayout {
                    spacing: 4

                    Text {
                        text: ""
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 11
                    }

                    Text {
                        text: "Storage"
                        color: Theme.palette.textMain
                        font.family: Theme.palette.font
                        font.pixelSize: 10
                        font.bold: true
                    }
                }

                Text {
                    text: root.storageUsedGiB.toFixed(1) + " / " + root.storageTotalGiB.toFixed(1) + " GiB"
                    color: root.metricColor(root.storagePercent)
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                }

                Text {
                    text: String(root.storagePercent) + "%"
                    color: root.metricColor(root.storagePercent)
                    font.family: Theme.palette.font
                    font.pixelSize: 13
                    font.bold: true
                }

                DashboardMetricProgressBar {
                    percent: root.storagePercent
                    fillColor: root.metricColor(root.storagePercent)
                }
            }
        }
    }
}
