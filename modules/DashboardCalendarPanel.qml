import QtQuick
import QtQuick.Layouts

import "../globals"

Rectangle {
    id: root

    property string title: "Calendar"
    property var calendarWeekdays: ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
    property int calendarCellWidth: 52
    property int calendarCellHeight: 36
    property int calendarGridGap: 3
    property int calendarWeekdayHeight: 30
    property int calendarWeekdayGap: 3
    property int calendarWeekdayToDatesGap: 2
    readonly property int calendarGridWidth: (calendarCellWidth * 7) + (calendarGridGap * 6)

    property var gridDayValueFn: null
    property var gridCellCurrentMonthFn: null
    property var gridCellIsTodayFn: null
    property var weekdayLabelFn: null

    signal previousYearRequested()
    signal previousMonthRequested()
    signal todayRequested()
    signal nextMonthRequested()
    signal nextYearRequested()

    property real swipeX: 0

    function dayValue(index) {
        return gridDayValueFn ? Number(gridDayValueFn(index)) : 0;
    }

    function inCurrentMonth(index) {
        return gridCellCurrentMonthFn ? !!gridCellCurrentMonthFn(index) : false;
    }

    function isToday(index) {
        return gridCellIsTodayFn ? !!gridCellIsTodayFn(index) : false;
    }

    function weekdayLabel(shortLabel) {
        return weekdayLabelFn ? String(weekdayLabelFn(shortLabel)) : String(shortLabel || "");
    }

    function animateSwipe(direction) {
        const dir = direction < 0 ? -1 : 1;
        swipeX = 42 * dir;
        calendarDatesAnimatedLayer.opacity = 0.75;
        calendarSwipeAnimation.restart();
    }

    implicitHeight: calendarColumn.implicitHeight + 8
    color: Theme.palette.bgMain
    radius: 10
    border.width: 1
    border.color: Theme.palette.borderInactive

    ColumnLayout {
        id: calendarColumn
        anchors.fill: parent
        anchors.margins: 4
        spacing: 0

        Text {
            text: root.title.length > 0 ? root.title : "Calendar"
            color: Theme.palette.textMain
            font.family: Theme.palette.font
            font.pixelSize: 16
            font.bold: true
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
            Layout.topMargin: 2
            Layout.bottomMargin: 8
        }

        Item {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: root.calendarGridWidth
            Layout.minimumWidth: root.calendarGridWidth
            Layout.maximumWidth: root.calendarGridWidth
            implicitHeight: root.calendarWeekdayHeight + root.calendarWeekdayToDatesGap + (root.calendarCellHeight * 6) + (root.calendarGridGap * 5)

            GridLayout {
                anchors.top: parent.top
                anchors.horizontalCenter: parent.horizontalCenter
                columns: 7
                rowSpacing: 0
                columnSpacing: root.calendarWeekdayGap
                width: root.calendarGridWidth

                Rectangle {
                    width: (root.calendarCellWidth * 7) + (6 * root.calendarWeekdayGap)
                    height: root.calendarWeekdayHeight
                    radius: 8
                    color: Theme.palette.bgWidget
                    clip: true

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        height: 1
                        color: Theme.palette.secondary
                    }

                    Row {
                        anchors.fill: parent
                        spacing: root.calendarWeekdayGap

                        Repeater {
                            model: root.calendarWeekdays
                            delegate: Rectangle {
                                required property var modelData
                                width: root.calendarCellWidth
                                height: root.calendarWeekdayHeight
                                color: "transparent"

                                Text {
                                    anchors.fill: parent
                                    anchors.margins: 3
                                    text: root.weekdayLabel(modelData)
                                    color: Theme.palette.textMain
                                    font.family: Theme.palette.font
                                    font.pixelSize: 11
                                    font.bold: true
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                }
                            }
                        }
                    }
                }
            }

            Item {
                id: calendarDatesAnimatedLayer
                anchors.top: parent.top
                anchors.topMargin: root.calendarWeekdayHeight + root.calendarWeekdayToDatesGap
                anchors.horizontalCenter: parent.horizontalCenter
                width: root.calendarGridWidth
                height: (root.calendarCellHeight * 6) + (root.calendarGridGap * 5)
                transform: Translate {
                    x: root.swipeX
                }

                Behavior on opacity {
                    NumberAnimation {
                        duration: 120
                        easing.type: Easing.OutCubic
                    }
                }

                GridLayout {
                    anchors.fill: parent
                    columns: 7
                    rowSpacing: root.calendarGridGap
                    columnSpacing: root.calendarGridGap

                    Repeater {
                        model: 42
                        delegate: Rectangle {
                            property int dayValue: root.dayValue(index)
                            property bool inCurrentMonth: root.inCurrentMonth(index)
                            property bool isToday: root.isToday(index)

                            implicitWidth: root.calendarCellWidth
                            implicitHeight: root.calendarCellHeight
                            radius: 6
                            color: inCurrentMonth ? Theme.palette.bgWidget : Theme.palette.bgMain
                            border.width: isToday ? 1 : 0
                            border.color: isToday ? Theme.palette.borderActive : Theme.palette.borderInactive

                            Text {
                                anchors.centerIn: parent
                                text: String(dayValue)
                                color: inCurrentMonth ? Theme.palette.textMain : Theme.palette.textMuted
                                font.family: Theme.palette.font
                                font.pixelSize: 12
                                font.bold: isToday
                            }
                        }
                    }
                }
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: root.calendarGridWidth
            Layout.minimumWidth: root.calendarGridWidth
            Layout.maximumWidth: root.calendarGridWidth
            Layout.topMargin: 5
            spacing: 6

            Rectangle {
                Layout.preferredWidth: 38
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: prevYearButton.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "󰅁󰅁"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    font.bold: true
                }

                MouseArea {
                    id: prevYearButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: {
                        root.previousYearRequested();
                        root.animateSwipe(-1);
                    }
                }
            }

            Rectangle {
                Layout.preferredWidth: 38
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: prevMonthButton.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "󰅁"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                    font.bold: true
                }

                MouseArea {
                    id: prevMonthButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: {
                        root.previousMonthRequested();
                        root.animateSwipe(-1);
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: todayButton.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "Today"
                    color: Theme.palette.secondary
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    font.bold: true
                }

                MouseArea {
                    id: todayButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: {
                        root.todayRequested();
                        root.animateSwipe(1);
                    }
                }
            }

            Rectangle {
                Layout.preferredWidth: 38
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: nextMonthButton.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "󰅂"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 12
                    font.bold: true
                }

                MouseArea {
                    id: nextMonthButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: {
                        root.nextMonthRequested();
                        root.animateSwipe(1);
                    }
                }
            }

            Rectangle {
                Layout.preferredWidth: 38
                Layout.preferredHeight: 28
                radius: 6
                color: Theme.palette.bgWidget
                border.width: 1
                border.color: nextYearButton.containsMouse ? Theme.palette.borderActive : Theme.palette.borderInactive

                Text {
                    anchors.centerIn: parent
                    text: "󰅂󰅂"
                    color: Theme.palette.textMain
                    font.family: Theme.palette.font
                    font.pixelSize: 11
                    font.bold: true
                }

                MouseArea {
                    id: nextYearButton
                    hoverEnabled: true
                    anchors.fill: parent
                    onClicked: {
                        root.nextYearRequested();
                        root.animateSwipe(1);
                    }
                }
            }
        }
    }

    ParallelAnimation {
        id: calendarSwipeAnimation

        NumberAnimation {
            target: root
            property: "swipeX"
            to: 0
            duration: 180
            easing.type: Easing.OutCubic
        }

        NumberAnimation {
            target: calendarDatesAnimatedLayer
            property: "opacity"
            to: 1
            duration: 180
            easing.type: Easing.OutCubic
        }
    }
}
